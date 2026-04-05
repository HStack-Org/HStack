use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Network error: {0}")]
    Network(String),
    #[error("API error (status {status}): {body}")]
    Api { status: u16, body: String },
    #[error("Provider error: {0}")]
    Provider(String),
    #[error("Provider contract error: {0}")]
    ProviderContract(String),
    #[error("Configuration error: {0}")]
    Configuration(String),
    #[error("Invariant violation: {0}")]
    Invariant(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Sandbox error: {0}")]
    Sandbox(String),
    #[error("Header error: {0}")]
    Header(String),
    #[error("HStack world error: {0}")]
    World(String),
    #[error("Max iterations reached")]
    MaxIterations,
    #[error("Rate limit exceeded. Wait {wait_time}s")]
    RateLimitExceeded { wait_time: f64 },
    #[error("Redis error: {0}")]
    Redis(String),
    #[error("Control system denied action: {0}")]
    Denied(String),
}
