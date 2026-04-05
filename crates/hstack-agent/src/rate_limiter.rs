use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use async_trait::async_trait;
use crate::error::Error;
use rand::Rng;
pub use hstack_core::provider::RateLimitConfig;

#[async_trait]
pub trait RateLimiter: Send + Sync {
    /// Acquires a slot for a request. Blocks (sleeps) if traffic shaping is required.
    async fn acquire(&self, provider_id: &str, request_cost: u32, token_cost: u32, config: &RateLimitConfig) -> Result<(), Error>;
}

/// A Redis-backed distributed rate limiter using the Virtual Queuing algorithm.
pub struct RedisRateLimiter {
    client: redis::Client,
    max_queue_delay: f64, 
}

impl RedisRateLimiter {
    pub fn new(redis_url: &str) -> Result<Self, Error> {
        let client = redis::Client::open(redis_url).map_err(|e| Error::Redis(e.to_string()))?;
        Ok(Self {
            client,
            max_queue_delay: 30.0 * 60.0,
        })
    }

    const LUA_SCRIPT: &'static str = r#"
        local redis_time = redis.call('TIME')
        local current_time = tonumber(redis_time[1]) + (tonumber(redis_time[2]) / 1000000)

        local rps_limit = tonumber(ARGV[1])
        local rpm_limit = tonumber(ARGV[2])
        local tpm_limit = tonumber(ARGV[3])
        local request_cost = tonumber(ARGV[4])
        local token_cost = tonumber(ARGV[5])
        local max_allowed_delay = tonumber(ARGV[6])

        local keys = { KEYS[1], KEYS[2], KEYS[3] }
        local limits = { rps_limit, rpm_limit, tpm_limit }
        local costs = { request_cost, request_cost, token_cost }
        local windows = { 1, 60, 60 }

        local highest_booked_time = current_time
        local actions = {}

        for i = 1, 3 do
            local limit = limits[i]
            if limit > 0 then
                local key = keys[i]
                local rate = limit / windows[i]
                local cost = costs[i]
                local wait_per_unit = cost / rate
                
                local booked_until = tonumber(redis.call('GET', key) or current_time)
                if booked_until < current_time then booked_until = current_time end
                
                if booked_until > highest_booked_time then
                    highest_booked_time = booked_until
                end
                
                local new_booked_until = booked_until + wait_per_unit
                table.insert(actions, {key = key, val = new_booked_until, ttl = math.ceil(new_booked_until - current_time + windows[i])})
            end
        end

        local wait_time = highest_booked_time - current_time
        if wait_time > max_allowed_delay then
            return {0, tostring(wait_time)}
        end

        for _, action in ipairs(actions) do
            redis.call('SET', action.key, tostring(action.val))
            redis.call('EXPIRE', action.key, action.ttl)
        end

        return {1, tostring(wait_time)}
    "#;
}

#[async_trait]
impl RateLimiter for RedisRateLimiter {
    async fn acquire(&self, provider_id: &str, request_cost: u32, token_cost: u32, config: &RateLimitConfig) -> Result<(), Error> {
        let mut conn = self.client.get_multiplexed_async_connection().await.map_err(|e| Error::Redis(e.to_string()))?;
        
        let keys = vec![
            format!("rl:prov:{provider_id}:batch:rps"),
            format!("rl:prov:{provider_id}:batch:rpm"),
            format!("rl:prov:{provider_id}:batch:tpm"),
        ];

        let args = vec![
            config.requests_per_second.to_string(),
            config.requests_per_minute.to_string(),
            config.tokens_per_minute.to_string(),
            request_cost.to_string(),
            token_cost.to_string(),
            self.max_queue_delay.to_string(),
        ];

        let script = redis::Script::new(Self::LUA_SCRIPT);
        let result: Vec<String> = script
            .key(&keys)
            .arg(&args)
            .invoke_async(&mut conn)
            .await
            .map_err(|e| Error::Redis(e.to_string()))?;

        if result.len() < 2 {
            return Err(Error::Redis("Malformed redis response".to_string()));
        }
        
        let allowed = result[0] == "1";
        let wait_time: f64 = result[1].parse().unwrap_or(0.0);

        if !allowed {
            return Err(Error::RateLimitExceeded { wait_time });
        }

        if wait_time > 0.0 {
            let jitter = rand::thread_rng().gen_range(0.01..0.1);
            tokio::time::sleep(Duration::from_secs_f64(wait_time + jitter)).await;
        }

        Ok(())
    }
}

/// In-process fallback using the SAME algorithm. 
/// Used when Redis is unavailable or for local-only execution.
pub struct LocalRateLimiter {
    pub(crate) state: Arc<Mutex<HashMap<String, f64>>>, // key -> booked_until timestamp
    max_queue_delay: f64,
}

impl Default for LocalRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalRateLimiter {
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(HashMap::new())),
            max_queue_delay: 30.0 * 60.0,
        }
    }

    fn now_f64() -> f64 {
        match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(d) => d.as_secs_f64(),
            Err(_) => 0.0,
        }
    }
}

#[async_trait]
impl RateLimiter for LocalRateLimiter {
    async fn acquire(&self, provider_id: &str, request_cost: u32, token_cost: u32, config: &RateLimitConfig) -> Result<(), Error> {
        let now = Self::now_f64();
        let mut state = self.state.lock().await;

        let mut highest_booked_time = now;
        let mut updates = Vec::new();

        let limits = [
            (format!("rl:prov:{provider_id}:batch:rps"), config.requests_per_second as f64, 1.0, request_cost as f64),
            (format!("rl:prov:{provider_id}:batch:rpm"), config.requests_per_minute as f64, 60.0, request_cost as f64),
            (format!("rl:prov:{provider_id}:batch:tpm"), config.tokens_per_minute as f64, 60.0, token_cost as f64),
        ];

        for (key, limit, window, cost) in limits {
            if limit > 0.0 {
                let booked_until = *state.get(&key).unwrap_or(&now);
                let current_booked = if booked_until < now { now } else { booked_until };

                if current_booked > highest_booked_time {
                    highest_booked_time = current_booked;
                }

                let rate = limit / window;
                let wait_per_unit = cost / rate;
                updates.push((key, current_booked + wait_per_unit));
            }
        }

        let wait_time = highest_booked_time - now;
        if wait_time > self.max_queue_delay {
            return Err(Error::RateLimitExceeded { wait_time });
        }

        // Commit updates to the local map
        for (key, new_time) in updates {
            state.insert(key, new_time);
        }

        if wait_time > 0.0 {
            let jitter = rand::thread_rng().gen_range(0.01..0.1);
            tokio::time::sleep(Duration::from_secs_f64(wait_time + jitter)).await;
        }

        Ok(())
    }
}
