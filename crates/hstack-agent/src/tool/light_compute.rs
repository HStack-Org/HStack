use async_trait::async_trait;
use deno_core::{op2, v8, JsRuntime, RuntimeOptions};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::HStackWorld;
use crate::tool::Tool;

#[op2(fast)]
fn op_lc_add(a: f64, b: f64) -> f64 {
    a + b
}

#[op2(fast)]
fn op_lc_sub(a: f64, b: f64) -> f64 {
    a - b
}

#[op2(fast)]
fn op_lc_mul(a: f64, b: f64) -> f64 {
    a * b
}

#[op2(fast)]
fn op_lc_div(a: f64, b: f64) -> Result<f64, deno_core::error::AnyError> {
    if b == 0.0 {
        return Err(deno_core::error::custom_error(
            "DivisionByZero",
            "division by zero",
        ));
    }
    Ok(a / b)
}

#[op2(fast)]
fn op_lc_pow(a: f64, b: f64) -> f64 {
    a.powf(b)
}

#[op2(fast)]
fn op_lc_sqrt(x: f64) -> Result<f64, deno_core::error::AnyError> {
    if x < 0.0 {
        return Err(deno_core::error::custom_error(
            "DomainError",
            "sqrt domain error: input must be >= 0",
        ));
    }
    Ok(x.sqrt())
}

#[op2(fast)]
fn op_lc_abs(x: f64) -> f64 {
    x.abs()
}

#[op2(fast)]
fn op_lc_round(x: f64) -> f64 {
    x.round()
}

#[op2(fast)]
fn op_lc_floor(x: f64) -> f64 {
    x.floor()
}

#[op2(fast)]
fn op_lc_ceil(x: f64) -> f64 {
    x.ceil()
}

#[op2(fast)]
fn op_lc_clamp(x: f64, lo: f64, hi: f64) -> Result<f64, deno_core::error::AnyError> {
    if lo > hi {
        return Err(deno_core::error::custom_error(
            "RangeError",
            "clamp range error: lo must be <= hi",
        ));
    }
    Ok(x.clamp(lo, hi))
}

#[op2]
#[string]
fn op_lc_upper(#[string] s: String) -> String {
    s.to_uppercase()
}

#[op2]
#[string]
fn op_lc_lower(#[string] s: String) -> String {
    s.to_lowercase()
}

#[op2]
#[string]
fn op_lc_trim(#[string] s: String) -> String {
    s.trim().to_string()
}

#[op2]
#[string]
fn op_lc_replace_all(#[string] s: String, #[string] from: String, #[string] to: String) -> String {
    s.replace(&from, &to)
}

deno_core::extension!(
    hstack_light_compute_ext,
    ops = [
        op_lc_add,
        op_lc_sub,
        op_lc_mul,
        op_lc_div,
        op_lc_pow,
        op_lc_sqrt,
        op_lc_abs,
        op_lc_round,
        op_lc_floor,
        op_lc_ceil,
        op_lc_clamp,
        op_lc_upper,
        op_lc_lower,
        op_lc_trim,
        op_lc_replace_all,
    ],
);

#[derive(Clone, Copy)]
struct LightComputeLimits {
    timeout_ms: u64,
    max_heap_bytes: usize,
    max_code_bytes: usize,
    max_input_bytes: usize,
    max_output_bytes: usize,
}

impl Default for LightComputeLimits {
    fn default() -> Self {
        Self {
            timeout_ms: 1_000,
            max_heap_bytes: 10 * 1024 * 1024,
            max_code_bytes: 20_000,
            max_input_bytes: 64_000,
            max_output_bytes: 64_000,
        }
    }
}

struct LightComputeOutput {
    result: Value,
    elapsed_ms: u128,
    timed_out: bool,
    oom: bool,
}

const LIGHT_COMPUTE_BOOTSTRAP: &str = r#"(() => {
  if (!globalThis.Deno || !Deno.core || !Deno.core.ops) {
    throw new Error('runtime ops unavailable');
  }
  const __ops = Deno.core.ops;
  globalThis.hstack = Object.freeze({
    add: (a, b) => __ops.op_lc_add(a, b),
    sub: (a, b) => __ops.op_lc_sub(a, b),
    mul: (a, b) => __ops.op_lc_mul(a, b),
    div: (a, b) => __ops.op_lc_div(a, b),
    pow: (a, b) => __ops.op_lc_pow(a, b),
    sqrt: (x) => __ops.op_lc_sqrt(x),
    abs: (x) => __ops.op_lc_abs(x),
    round: (x) => __ops.op_lc_round(x),
    floor: (x) => __ops.op_lc_floor(x),
    ceil: (x) => __ops.op_lc_ceil(x),
    clamp: (x, lo, hi) => __ops.op_lc_clamp(x, lo, hi),
    upper: (s) => __ops.op_lc_upper(String(s)),
    lower: (s) => __ops.op_lc_lower(String(s)),
    trim: (s) => __ops.op_lc_trim(String(s)),
    replaceAll: (s, from, to) => __ops.op_lc_replace_all(String(s), String(from), String(to)),

    sum: (arr) => {
      if (!Array.isArray(arr)) throw new Error('sum expects array');
      return arr.reduce((acc, x) => acc + Number(x), 0);
    },
    mean: (arr) => {
      if (!Array.isArray(arr) || arr.length === 0) throw new Error('mean expects non-empty array');
      return arr.reduce((acc, x) => acc + Number(x), 0) / arr.length;
    },
    median: (arr) => {
      if (!Array.isArray(arr) || arr.length === 0) throw new Error('median expects non-empty array');
      const sorted = arr.map(Number).sort((a, b) => a - b);
      const mid = Math.floor(sorted.length / 2);
      return sorted.length % 2 === 0 ? (sorted[mid - 1] + sorted[mid]) / 2 : sorted[mid];
    },
    min: (arr) => {
      if (!Array.isArray(arr) || arr.length === 0) throw new Error('min expects non-empty array');
      return Math.min(...arr.map(Number));
    },
    max: (arr) => {
      if (!Array.isArray(arr) || arr.length === 0) throw new Error('max expects non-empty array');
      return Math.max(...arr.map(Number));
    },
    getPath: (obj, path) => {
      if (typeof path !== 'string' || path.length === 0) return obj;
      return path.split('.').reduce((acc, key) => (acc == null ? undefined : acc[key]), obj);
    },
    pick: (obj, keys) => {
      if (obj == null || typeof obj !== 'object') throw new Error('pick expects object');
      if (!Array.isArray(keys)) throw new Error('pick expects keys array');
      const out = {};
      for (const k of keys) {
        if (Object.prototype.hasOwnProperty.call(obj, k)) out[k] = obj[k];
      }
      return out;
    },
  });

  globalThis.Deno = undefined;
})();"#;

fn forbidden_construct_reason(code: &str) -> Option<&'static str> {
    let lower = code.to_ascii_lowercase();
    let checks = [
        ("import ", "imports are disabled"),
        ("import(", "dynamic imports are disabled"),
        ("require(", "require() is disabled"),
        ("fetch(", "network access is disabled"),
        ("websocket", "network access is disabled"),
        ("worker(", "workers are disabled"),
        ("process.", "process access is disabled"),
        ("deno.", "runtime access is disabled"),
    ];

    for (needle, reason) in checks {
        if lower.contains(needle) {
            return Some(reason);
        }
    }
    None
}

fn classify_oom(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("out of memory") || lower.contains("heap") || lower.contains("allocation failed")
}

fn run_light_compute(code: &str, input: Value, limits: LightComputeLimits) -> Result<LightComputeOutput, Error> {
    if code.len() > limits.max_code_bytes {
        return Err(Error::Internal(format!(
            "light_compute code exceeds {} bytes",
            limits.max_code_bytes
        )));
    }
    if let Some(reason) = forbidden_construct_reason(code) {
        return Err(Error::Internal(format!("light_compute rejected source: {reason}")));
    }

    let input_json = serde_json::to_string(&input)
        .map_err(|e| Error::Internal(format!("Failed to serialize light_compute input: {e}")))?;
    if input_json.len() > limits.max_input_bytes {
        return Err(Error::Internal(format!(
            "light_compute input exceeds {} bytes",
            limits.max_input_bytes
        )));
    }

    let started = Instant::now();

    let create_params = v8::CreateParams::default().heap_limits(0, limits.max_heap_bytes);
    let mut runtime = JsRuntime::new(RuntimeOptions {
        create_params: Some(create_params),
        extensions: vec![hstack_light_compute_ext::init_ops_and_esm()],
        ..Default::default()
    });

    let terminated = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));

    let isolate_handle = runtime.v8_isolate().thread_safe_handle();
    let timeout_flag = Arc::clone(&terminated);
    let done_flag = Arc::clone(&done);
    let timeout_ms = limits.timeout_ms;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(timeout_ms));
        if !done_flag.load(Ordering::SeqCst) {
            timeout_flag.store(true, Ordering::SeqCst);
            isolate_handle.terminate_execution();
        }
    });

    runtime
        .execute_script("light_compute_bootstrap.js", LIGHT_COMPUTE_BOOTSTRAP)
        .map_err(|e| Error::Internal(format!("light_compute bootstrap error: {e}")))?;

    let wrapped = format!(
        "(() => {{\n  const input = JSON.parse({input_literal});\n  const __result = (() => {{\n{code}\n  }})();\n  return JSON.stringify(__result === undefined ? null : __result);\n}})()",
        input_literal = serde_json::to_string(&input_json)
            .map_err(|e| Error::Internal(format!("Failed to encode light_compute input literal: {e}")))?,
        code = code,
    );

    let executed = runtime.execute_script("light_compute.js", wrapped);
    done.store(true, Ordering::SeqCst);

    let timed_out = terminated.load(Ordering::SeqCst);
    if timed_out {
        return Ok(LightComputeOutput {
            result: Value::Null,
            elapsed_ms: started.elapsed().as_millis(),
            timed_out: true,
            oom: false,
        });
    }

    let global = executed.map_err(|e| {
        let msg = e.to_string();
        if classify_oom(&msg) {
            Error::Internal(format!("light_compute out of memory: {msg}"))
        } else {
            Error::Internal(format!("light_compute execution error: {msg}"))
        }
    })?;

    let json_payload = {
        let scope = &mut runtime.handle_scope();
        let local = global.open(scope);
        local.to_rust_string_lossy(scope)
    };
    if json_payload.len() > limits.max_output_bytes {
        return Err(Error::Internal(format!(
            "light_compute output exceeds {} bytes",
            limits.max_output_bytes
        )));
    }

    let result = serde_json::from_str::<Value>(&json_payload)
        .map_err(|e| Error::Internal(format!("light_compute returned non-JSON result: {e}")))?;

    Ok(LightComputeOutput {
        result,
        elapsed_ms: started.elapsed().as_millis(),
        timed_out: false,
        oom: false,
    })
}

/// Sandboxed compute for short JS snippets with strict resource limits.
pub struct LightComputeTool {
    limits: LightComputeLimits,
}

impl LightComputeTool {
    pub fn new() -> Self {
        Self {
            limits: LightComputeLimits::default(),
        }
    }
}

impl Default for LightComputeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for LightComputeTool {
    fn name(&self) -> &str {
        "light_compute"
    }

    fn description(&self) -> &str {
        "Runs short sandboxed JavaScript with strict limits and allowlisted helpers in global `hstack` (math, stats, strings, and object utilities)."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "JavaScript body executed inside an IIFE. Use return to produce output."
                },
                "input": {
                    "type": "object",
                    "description": "Structured JSON object exposed as variable 'input' in the script."
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld) -> Result<AgentAction, Error> {
        let code = args
            .get("code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Internal("light_compute requires non-empty 'code'".to_string()))?
            .to_string();

        let input = args.get("input").cloned().unwrap_or(Value::Null);
        let limits = self.limits;

        let output_res = tokio::task::spawn_blocking(move || run_light_compute(&code, input, limits))
            .await
            .map_err(|e| Error::Internal(format!("light_compute join error: {e}")));

        let event = match output_res {
            Ok(Ok(output)) if output.timed_out => serde_json::json!({
                "ok": false,
                "error": {
                    "type": "timeout",
                    "message": "light_compute exceeded timeout"
                },
                "metrics": {
                    "elapsed_ms": output.elapsed_ms,
                    "timeout_ms": self.limits.timeout_ms
                }
            }),
            Ok(Ok(output)) if output.oom => serde_json::json!({
                "ok": false,
                "error": {
                    "type": "oom",
                    "message": "light_compute memory limit exceeded"
                },
                "metrics": {
                    "elapsed_ms": output.elapsed_ms,
                    "max_heap_bytes": self.limits.max_heap_bytes
                }
            }),
            Ok(Ok(output)) => serde_json::json!({
                "ok": true,
                "result": output.result,
                "metrics": {
                    "elapsed_ms": output.elapsed_ms,
                    "timeout_ms": self.limits.timeout_ms,
                    "max_heap_bytes": self.limits.max_heap_bytes,
                    "max_output_bytes": self.limits.max_output_bytes
                }
            }),
            Ok(Err(e)) | Err(e) => {
                let msg = e.to_string();
                let lower = msg.to_ascii_lowercase();
                let err_type = if lower.contains("rejected source") {
                    "forbidden"
                } else if classify_oom(&msg) {
                    "oom"
                } else if lower.contains("output exceeds") {
                    "output_too_large"
                } else {
                    "runtime"
                };
                serde_json::json!({
                    "ok": false,
                    "error": {
                        "type": err_type,
                        "message": msg,
                    },
                    "limits": {
                        "timeout_ms": self.limits.timeout_ms,
                        "max_heap_bytes": self.limits.max_heap_bytes,
                        "max_code_bytes": self.limits.max_code_bytes,
                        "max_input_bytes": self.limits.max_input_bytes,
                        "max_output_bytes": self.limits.max_output_bytes
                    }
                })
            }
        };

        Ok(AgentAction::UpdateWorkingMemory(
            WorkingMemoryDelta::AddTechnicalNoise("light_compute".to_string(), event),
        ))
    }
}
