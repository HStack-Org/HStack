use async_trait::async_trait;
#[cfg(any(not(target_os = "android"), test))]
use deno_core::{op2, v8, JsRuntime, RuntimeOptions};
#[cfg(any(target_os = "android", test))]
use monty::{ExcType, JsonMontyObject, LimitedTracker, MontyObject, MontyRun, PrintWriter, ResourceLimits};
use serde_json::Value;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::WorkspaceDelta;

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_add(a: f64, b: f64) -> f64 {
    a + b
}

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_sub(a: f64, b: f64) -> f64 {
    a - b
}

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_mul(a: f64, b: f64) -> f64 {
    a * b
}

#[cfg(any(not(target_os = "android"), test))]
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

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_pow(a: f64, b: f64) -> f64 {
    a.powf(b)
}

#[cfg(any(not(target_os = "android"), test))]
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

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_abs(x: f64) -> f64 {
    x.abs()
}

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_round(x: f64) -> f64 {
    x.round()
}

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_floor(x: f64) -> f64 {
    x.floor()
}

#[cfg(any(not(target_os = "android"), test))]
#[op2(fast)]
fn op_lc_ceil(x: f64) -> f64 {
    x.ceil()
}

#[cfg(any(not(target_os = "android"), test))]
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

#[cfg(any(not(target_os = "android"), test))]
#[op2]
#[string]
fn op_lc_upper(#[string] s: String) -> String {
    s.to_uppercase()
}

#[cfg(any(not(target_os = "android"), test))]
#[op2]
#[string]
fn op_lc_lower(#[string] s: String) -> String {
    s.to_lowercase()
}

#[cfg(any(not(target_os = "android"), test))]
#[op2]
#[string]
fn op_lc_trim(#[string] s: String) -> String {
    s.trim().to_string()
}

#[cfg(any(not(target_os = "android"), test))]
#[op2]
#[string]
fn op_lc_replace_all(#[string] s: String, #[string] from: String, #[string] to: String) -> String {
    s.replace(&from, &to)
}

#[cfg(any(not(target_os = "android"), test))]
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LightComputeBackendKind {
    #[cfg(any(not(target_os = "android"), test))]
    V8,
    #[cfg(any(target_os = "android", test))]
    Monty,
}

impl LightComputeBackendKind {
    fn default_for_platform() -> Self {
        #[cfg(target_os = "android")]
        {
            Self::Monty
        }
        #[cfg(not(target_os = "android"))]
        {
            Self::V8
        }
    }

    fn description(self) -> &'static str {
        match self {
            #[cfg(any(not(target_os = "android"), test))]
            Self::V8 => {
                "Runs short sandboxed JavaScript with strict limits and allowlisted helpers in global `hstack` (math, stats, strings, and object utilities)."
            }
            #[cfg(any(target_os = "android", test))]
            Self::Monty => {
                "Runs short sandboxed Python on Monty with strict limits and allowlisted helpers in `hstack` (math, stats, strings, and object utilities)."
            }
        }
    }
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

#[cfg(any(target_os = "android", test))]
const MONTY_HSTACK_PRELUDE: &str = r#"
def hstack_add(a, b):
    return a + b

def hstack_sub(a, b):
    return a - b

def hstack_mul(a, b):
    return a * b

def hstack_div(a, b):
    if b == 0:
        raise ValueError('division by zero')
    return a / b

def hstack_pow(a, b):
    return a ** b

def hstack_sqrt(x):
    if x < 0:
        raise ValueError('sqrt domain error: input must be >= 0')
    return x ** 0.5

def hstack_abs(x):
    return abs(x)

def hstack_round(x):
    return round(x)

def hstack_floor(x):
    return int(x // 1)

def hstack_ceil(x):
    floor = int(x // 1)
    return floor if x == floor else floor + 1

def hstack_clamp(x, lo, hi):
    if lo > hi:
        raise ValueError('clamp range error: lo must be <= hi')
    if x < lo:
        return lo
    if x > hi:
        return hi
    return x

def hstack_upper(s):
    return str(s).upper()

def hstack_lower(s):
    return str(s).lower()

def hstack_trim(s):
    return str(s).strip()

def hstack_replaceAll(s, old, new):
    return str(s).replace(str(old), str(new))

def hstack_sum(arr):
    if type(arr) != list:
        raise ValueError('sum expects array')
    total = 0
    for item in arr:
        total = total + item
    return total

def hstack_mean(arr):
    if type(arr) != list or len(arr) == 0:
        raise ValueError('mean expects non-empty array')
    return hstack_sum(arr) / len(arr)

def hstack_median(arr):
    if type(arr) != list or len(arr) == 0:
        raise ValueError('median expects non-empty array')
    sorted_values = sorted(arr)
    mid = len(sorted_values) // 2
    if len(sorted_values) % 2 == 0:
        return (sorted_values[mid - 1] + sorted_values[mid]) / 2
    return sorted_values[mid]

def hstack_min(arr):
    if type(arr) != list or len(arr) == 0:
        raise ValueError('min expects non-empty array')
    return min(arr)

def hstack_max(arr):
    if type(arr) != list or len(arr) == 0:
        raise ValueError('max expects non-empty array')
    return max(arr)

def hstack_getPath(obj, path):
    if type(path) != str or path == '':
        return obj
    current = obj
    for key in path.split('.'):
        if current is None:
            return None
        if type(current) != dict or key not in current:
            return None
        current = current[key]
    return current

def hstack_pick(obj, keys):
    if type(obj) != dict:
        raise ValueError('pick expects object')
    if type(keys) != list:
        raise ValueError('pick expects keys array')
    out = {}
    for key in keys:
        if key in obj:
            out[key] = obj[key]
    return out
"#;

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
    lower.contains("out of memory")
        || lower.contains("heap")
        || lower.contains("allocation failed")
        || lower.contains("memory limit")
}

#[cfg(any(target_os = "android", test))]
fn replace_token(input: &str, token: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let token_chars: Vec<char> = token.chars().collect();
    let mut index = 0;

    while index < chars.len() {
        let matches_token = index + token_chars.len() <= chars.len()
            && chars[index..index + token_chars.len()] == token_chars[..]
            && (index == 0 || !(chars[index - 1].is_ascii_alphanumeric() || chars[index - 1] == '_'))
            && (index + token_chars.len() == chars.len()
                || !(chars[index + token_chars.len()].is_ascii_alphanumeric()
                    || chars[index + token_chars.len()] == '_'));

        if matches_token {
            out.push_str(replacement);
            index += token_chars.len();
        } else {
            out.push(chars[index]);
            index += 1;
        }
    }

    out
}

#[cfg(any(target_os = "android", test))]
fn convert_input_accesses(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        let starts_chain = index + 6 <= chars.len()
            && chars[index..index + 6] == ['i', 'n', 'p', 'u', 't', '.']
            && (index == 0 || !(chars[index - 1].is_ascii_alphanumeric() || chars[index - 1] == '_'));

        if !starts_chain {
            out.push(chars[index]);
            index += 1;
            continue;
        }

        let mut cursor = index + 5;
        let mut segments = Vec::new();
        while cursor < chars.len() && chars[cursor] == '.' {
            cursor += 1;
            let segment_start = cursor;
            while cursor < chars.len() && (chars[cursor].is_ascii_alphanumeric() || chars[cursor] == '_') {
                cursor += 1;
            }
            if segment_start == cursor {
                break;
            }
            let segment: String = chars[segment_start..cursor].iter().collect();
            segments.push(segment);
        }

        if segments.is_empty() {
            out.push(chars[index]);
            index += 1;
            continue;
        }

        out.push_str("input");
        for segment in segments {
            out.push_str("[");
            out.push_str(&serde_json::to_string(&segment).unwrap_or_else(|_| format!("\"{segment}\"")));
            out.push(']');
        }
        index = cursor;
    }

    out
}

#[cfg(any(target_os = "android", test))]
fn split_top_level(input: &str, separator: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth_paren = 0usize;
    let mut depth_bracket = 0usize;
    let mut depth_brace = 0usize;
    let mut quote: Option<char> = None;
    let mut escape = false;

    for ch in input.chars() {
        if let Some(active_quote) = quote {
            current.push(ch);
            if escape {
                escape = false;
                continue;
            }
            if ch == '\\' {
                escape = true;
            } else if ch == active_quote {
                quote = None;
            }
            continue;
        }

        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                current.push(ch);
            }
            '(' => {
                depth_paren += 1;
                current.push(ch);
            }
            ')' => {
                depth_paren = depth_paren.saturating_sub(1);
                current.push(ch);
            }
            '[' => {
                depth_bracket += 1;
                current.push(ch);
            }
            ']' => {
                depth_bracket = depth_bracket.saturating_sub(1);
                current.push(ch);
            }
            '{' => {
                depth_brace += 1;
                current.push(ch);
            }
            '}' => {
                depth_brace = depth_brace.saturating_sub(1);
                current.push(ch);
            }
            _ if ch == separator && depth_paren == 0 && depth_bracket == 0 && depth_brace == 0 => {
                parts.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }

    if !current.trim().is_empty() {
        parts.push(current.trim().to_string());
    }

    parts
}

#[cfg(any(target_os = "android", test))]
fn translate_js_expression(expr: &str) -> Result<String, Error> {
    let trimmed = expr.trim().trim_end_matches(';').trim();
    let normalized = replace_token(
        &replace_token(&replace_token(trimmed, "true", "True"), "false", "False"),
        "null",
        "None",
    );
    let normalized = convert_input_accesses(&normalized).replace("hstack.", "hstack_");

    if normalized.starts_with('{') && normalized.ends_with('}') {
        let inner = &normalized[1..normalized.len() - 1];
        if inner.trim().is_empty() {
            return Ok("{}".to_string());
        }

        let mut entries = Vec::new();
        for entry in split_top_level(inner, ',') {
            let mut pair = split_top_level(&entry, ':');
            if pair.len() < 2 {
                return Err(Error::Provider(format!(
                    "light_compute could not translate object literal entry: {entry}"
                )));
            }
            let value = pair.split_off(1).join(":");
            let key = pair.remove(0).trim().to_string();
            let key_literal = if key.starts_with('"') || key.starts_with('\'') {
                key
            } else {
                serde_json::to_string(&key)
                    .map_err(|e| Error::Serialization(format!("Failed to encode object key: {e}")))?
            };
            let value_literal = translate_js_expression(&value)?;
            entries.push(format!("{key_literal}: {value_literal}"));
        }
        return Ok(format!("{{{}}}", entries.join(", ")));
    }

    Ok(normalized)
}

#[cfg(any(target_os = "android", test))]
fn translate_js_body_to_monty(code: &str) -> Result<String, Error> {
    let trimmed = code.trim();
    if trimmed == "while (true) {}" || trimmed == "while(true){}" {
        return Ok("while True:\n        pass".to_string());
    }

    let mut translated_lines = Vec::new();
    for raw_line in trimmed.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("return ") {
            translated_lines.push(format!("return {}", translate_js_expression(rest)?));
            continue;
        }
        if let Some(rest) = line.strip_prefix("const ") {
            let Some((name, expr)) = rest.split_once('=') else {
                return Err(Error::Provider("light_compute could not translate const assignment".to_string()));
            };
            translated_lines.push(format!(
                "{} = {}",
                name.trim(),
                translate_js_expression(expr)?
            ));
            continue;
        }
        if let Some(rest) = line.strip_prefix("let ") {
            let Some((name, expr)) = rest.split_once('=') else {
                return Err(Error::Provider("light_compute could not translate let assignment".to_string()));
            };
            translated_lines.push(format!(
                "{} = {}",
                name.trim(),
                translate_js_expression(expr)?
            ));
            continue;
        }
        translated_lines.push(translate_js_expression(line)?);
    }

    if translated_lines.is_empty() {
        return Err(Error::Provider("light_compute requires non-empty 'code'".to_string()));
    }

    let mut out = String::from("def __hstack_light_compute_main():\n");
    for line in translated_lines {
        out.push_str("    ");
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("__hstack_light_compute_main()\n");
    Ok(out)
}

#[cfg(any(target_os = "android", test))]
fn json_to_monty(value: Value) -> Result<MontyObject, Error> {
    match value {
        Value::Null => Ok(MontyObject::None),
        Value::Bool(flag) => Ok(MontyObject::Bool(flag)),
        Value::Number(number) => {
            if let Some(int) = number.as_i64() {
                Ok(MontyObject::Int(int))
            } else if let Some(float) = number.as_f64() {
                Ok(MontyObject::Float(float))
            } else {
                Err(Error::Serialization(format!(
                    "light_compute cannot convert JSON number {number} to Monty input"
                )))
            }
        }
        Value::String(text) => Ok(MontyObject::String(text)),
        Value::Array(items) => items
            .into_iter()
            .map(json_to_monty)
            .collect::<Result<Vec<_>, _>>()
            .map(MontyObject::List),
        Value::Object(map) => {
            let mut pairs = Vec::with_capacity(map.len());
            for (key, value) in map {
                pairs.push((MontyObject::String(key), json_to_monty(value)?));
            }
            Ok(MontyObject::dict(pairs))
        }
    }
}

#[cfg(any(target_os = "android", test))]
fn run_light_compute_monty(code: &str, input: Value, limits: LightComputeLimits) -> Result<LightComputeOutput, Error> {
    if code.len() > limits.max_code_bytes {
        return Err(Error::Sandbox(format!(
            "light_compute code exceeds {} bytes",
            limits.max_code_bytes
        )));
    }
    if let Some(reason) = forbidden_construct_reason(code) {
        return Err(Error::Sandbox(format!("light_compute rejected source: {reason}")));
    }

    let input_json = serde_json::to_string(&input)
        .map_err(|e| Error::Serialization(format!("Failed to serialize light_compute input: {e}")))?;
    if input_json.len() > limits.max_input_bytes {
        return Err(Error::Sandbox(format!(
            "light_compute input exceeds {} bytes",
            limits.max_input_bytes
        )));
    }

    let translated_body = translate_js_body_to_monty(code)?;
    let program = format!("{MONTY_HSTACK_PRELUDE}\n{translated_body}");
    let started = Instant::now();

    let runner = MontyRun::new(program, "light_compute.py", vec!["input".to_string()])
        .map_err(|e| Error::Sandbox(format!("light_compute compile error: {e}")))?;
    let tracker = LimitedTracker::new(
        ResourceLimits::new()
            .max_duration(Duration::from_millis(limits.timeout_ms))
            .max_memory(limits.max_heap_bytes),
    );
    let result = runner.run(vec![json_to_monty(input)?], tracker, PrintWriter::Stdout);

    match result {
        Ok(result) => {
            let json_payload = serde_json::to_string(&JsonMontyObject(&result))
                .map_err(|e| Error::Serialization(format!("light_compute failed to encode Monty output: {e}")))?;
            if json_payload.len() > limits.max_output_bytes {
                return Err(Error::Sandbox(format!(
                    "light_compute output exceeds {} bytes",
                    limits.max_output_bytes
                )));
            }
            let value = serde_json::from_str::<Value>(&json_payload)
                .map_err(|e| Error::Serialization(format!("light_compute returned non-JSON result: {e}")))?;
            Ok(LightComputeOutput {
                result: value,
                elapsed_ms: started.elapsed().as_millis(),
                timed_out: false,
                oom: false,
            })
        }
        Err(error) if error.exc_type() == ExcType::TimeoutError => Ok(LightComputeOutput {
            result: Value::Null,
            elapsed_ms: started.elapsed().as_millis(),
            timed_out: true,
            oom: false,
        }),
        Err(error) => {
            let message = error.to_string();
            if classify_oom(&message) {
                Err(Error::Sandbox(format!("light_compute out of memory: {message}")))
            } else {
                Err(Error::Sandbox(format!("light_compute execution error: {message}")))
            }
        }
    }
}

#[cfg(any(not(target_os = "android"), test))]
fn run_light_compute_v8(code: &str, input: Value, limits: LightComputeLimits) -> Result<LightComputeOutput, Error> {
    if code.len() > limits.max_code_bytes {
        return Err(Error::Sandbox(format!(
            "light_compute code exceeds {} bytes",
            limits.max_code_bytes
        )));
    }
    if let Some(reason) = forbidden_construct_reason(code) {
        return Err(Error::Sandbox(format!("light_compute rejected source: {reason}")));
    }

    let input_json = serde_json::to_string(&input)
        .map_err(|e| Error::Serialization(format!("Failed to serialize light_compute input: {e}")))?;
    if input_json.len() > limits.max_input_bytes {
        return Err(Error::Sandbox(format!(
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
        .map_err(|e| Error::Invariant(format!("light_compute bootstrap error: {e}")))?;

    let wrapped = format!(
        "(() => {{\n  const input = JSON.parse({input_literal});\n  const __result = (() => {{\n{code}\n  }})();\n  return JSON.stringify(__result === undefined ? null : __result);\n}})()",
        input_literal = serde_json::to_string(&input_json)
            .map_err(|e| Error::Serialization(format!("Failed to encode light_compute input literal: {e}")))?,
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
            Error::Sandbox(format!("light_compute out of memory: {msg}"))
        } else {
            Error::Sandbox(format!("light_compute execution error: {msg}"))
        }
    })?;

    let json_payload = {
        let scope = &mut runtime.handle_scope();
        let local = global.open(scope);
        local.to_rust_string_lossy(scope)
    };
    if json_payload.len() > limits.max_output_bytes {
        return Err(Error::Sandbox(format!(
            "light_compute output exceeds {} bytes",
            limits.max_output_bytes
        )));
    }

    let result = serde_json::from_str::<Value>(&json_payload)
        .map_err(|e| Error::Serialization(format!("light_compute returned non-JSON result: {e}")))?;

    Ok(LightComputeOutput {
        result,
        elapsed_ms: started.elapsed().as_millis(),
        timed_out: false,
        oom: false,
    })
}

fn run_backend(
    backend: LightComputeBackendKind,
    code: &str,
    input: Value,
    limits: LightComputeLimits,
) -> Result<LightComputeOutput, Error> {
    match backend {
        #[cfg(any(not(target_os = "android"), test))]
        LightComputeBackendKind::V8 => run_light_compute_v8(code, input, limits),
        #[cfg(any(target_os = "android", test))]
        LightComputeBackendKind::Monty => run_light_compute_monty(code, input, limits),
    }
}

/// Sandboxed compute for short code snippets with strict resource limits.
pub struct LightComputeTool {
    backend: LightComputeBackendKind,
    limits: LightComputeLimits,
}

impl LightComputeTool {
    pub fn new() -> Self {
        Self {
            backend: LightComputeBackendKind::default_for_platform(),
            limits: LightComputeLimits::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_for_tests_with_backend(backend: &str) -> Self {
        let backend = match backend {
            #[cfg(any(not(target_os = "android"), test))]
            "v8" => LightComputeBackendKind::V8,
            #[cfg(any(target_os = "android", test))]
            "monty" => LightComputeBackendKind::Monty,
            _ => panic!("unknown test backend: {backend}"),
        };
        Self {
            backend,
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
        self.backend.description()
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Sandboxed code body. Use return to produce output. The runtime language depends on the active backend: JavaScript on desktop V8, Python on Android Monty."
                },
                "input": {
                    "type": "object",
                    "description": "Structured JSON object exposed as variable 'input' in the script."
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let code = args
            .get("code")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Provider("light_compute requires non-empty 'code'".to_string()))?
            .to_string();

        let input = match args.get("input") {
            None => Value::Null,
            Some(Value::Object(map)) => Value::Object(map.clone()),
            Some(Value::Null) => Value::Null,
            Some(_) => {
                return Err(Error::Provider(
                    "light_compute 'input' must be an object when provided".to_string(),
                ))
            }
        };
        let limits = self.limits;
        let backend = self.backend;

        let output_res = tokio::task::spawn_blocking(move || run_backend(backend, &code, input, limits))
            .await
            .map_err(|e| Error::Invariant(format!("light_compute join error: {e}")));

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

        let summary = if event.get("ok").and_then(Value::as_bool) == Some(true) {
            "light_compute success".to_string()
        } else {
            event
                .get("error")
                .and_then(|error| error.get("type"))
                .and_then(Value::as_str)
                .map(|kind| format!("light_compute {kind}"))
                .unwrap_or_else(|| "light_compute runtime".to_string())
        };

        Ok(AgentAction::Compound(vec![
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
                "light_compute".to_string(),
                event.clone(),
            )),
            AgentAction::UpdateWorkspace(WorkspaceDelta::RecordCompute {
                summary,
                payload: event,
            }),
        ]))
    }
}
