use async_trait::async_trait;
use hstack_core::execution::{
    ExecutionHandle, ExecutionInstruction, ExecutionLimits, ExecutionOutput, ExecutionState,
    ExecutionStatus, NetworkPolicy, OutputStreamingPolicy, RunToolRequest, StdinPolicy,
};
use hstack_core::filesystem::CapabilityProfile;
use hstack_core::virtual_fs::VirtualPath;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::{LightComputeTool, Tool};
use crate::workspace::WorkspaceDelta;

pub struct ExecutionRequestTool;

struct ExecutionRecord {
    status: ExecutionStatus,
    output: ExecutionOutput,
}

fn execution_store() -> &'static Mutex<HashMap<String, ExecutionRecord>> {
    static STORE: OnceLock<Mutex<HashMap<String, ExecutionRecord>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[async_trait]
impl Tool for ExecutionRequestTool {
    fn name(&self) -> &str {
        "execution_request"
    }

    fn description(&self) -> &str {
        "Runs, polls, collects, or cancels constrained execution requests without exposing any host-shell execution path. Public allowlist currently supports only internal light_compute execution."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["run_tool", "poll_execution", "collect_output", "cancel_execution"] },
                "tool_id": { "type": "string" },
                "argv": { "type": "array", "items": { "type": "string" } },
                "cwd": { "type": "string" },
                "env_allowlist": { "type": "array", "items": { "type": "string" } },
                "stdin_policy": { "type": "string", "enum": ["disabled", "empty"] },
                "network_policy": { "type": "string", "enum": ["deny"] },
                "output_streaming_policy": { "type": "string", "enum": ["buffered", "streaming"] },
                "filesystem_capability_profile": { "type": "string", "enum": ["safe_data_sandbox", "project_sandbox"] },
                "timeout_ms": { "type": "integer", "minimum": 1 },
                "max_output_bytes": { "type": "integer", "minimum": 1 },
                "handle": { "type": "string" }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: Value, world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("execution_request requires a string 'operation'".to_string()))?;

        let action = match operation {
            "run_tool" => {
                let request = parse_run_tool_request(&args)?;
                handle_run_tool(request, world, memory).await?
            }
            "poll_execution" => {
                let handle = parse_handle(&args)?;
                handle_poll_execution(handle)?
            }
            "collect_output" => {
                let handle = parse_handle(&args)?;
                handle_collect_output(handle)?
            }
            "cancel_execution" => {
                let handle = parse_handle(&args)?;
                handle_cancel_execution(handle)?
            }
            _ => {
                return Err(Error::Provider(
                    "execution_request received unsupported operation".to_string(),
                ))
            }
        };

        Ok(action)
    }
}

async fn handle_run_tool(
    request: RunToolRequest,
    world: &dyn HStackWorld,
    memory: &WorkingMemory,
) -> Result<AgentAction, Error> {
    let handle = ExecutionHandle(Uuid::new_v4().to_string());
    let record = run_allowlisted_tool(handle.clone(), request.clone(), world, memory).await;
    let event = serde_json::json!({
        "ok": matches!(record.status.state, ExecutionState::Completed),
        "instruction": ExecutionInstruction::RunTool(request.clone()),
        "status": record.status,
        "output": record.output,
    });

    let state_label = format!("{:?}", record.status.state).to_ascii_lowercase();
    let handle_key = handle.0.clone();
    execution_store()
        .lock()
        .map_err(|e| Error::Invariant(format!("execution store mutex poisoned: {e}")))?
        .insert(handle_key, record);

    Ok(AgentAction::Compound(vec![
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "execution_request".to_string(),
            event.clone(),
        )),
        AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
            summary: format!("execution {}", request.tool_id),
            state: state_label,
            detail: event,
        }),
    ]))
}

fn handle_poll_execution(handle: ExecutionHandle) -> Result<AgentAction, Error> {
    let store = execution_store()
        .lock()
        .map_err(|e| Error::Invariant(format!("execution store mutex poisoned: {e}")))?;
    let record = store.get(&handle.0).ok_or_else(|| {
        Error::Sandbox(format!("execution handle '{}' is unknown", handle.0))
    })?;
    let event = serde_json::json!({
        "instruction": ExecutionInstruction::PollExecution { handle: handle.clone() },
        "status": record.status,
    });

    Ok(AgentAction::Compound(vec![
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "execution_request".to_string(),
            event.clone(),
        )),
        AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
            summary: format!("poll {}", handle.0),
            state: format!("{:?}", record.status.state).to_ascii_lowercase(),
            detail: event,
        }),
    ]))
}

fn handle_collect_output(handle: ExecutionHandle) -> Result<AgentAction, Error> {
    let store = execution_store()
        .lock()
        .map_err(|e| Error::Invariant(format!("execution store mutex poisoned: {e}")))?;
    let record = store.get(&handle.0).ok_or_else(|| {
        Error::Sandbox(format!("execution handle '{}' is unknown", handle.0))
    })?;
    let event = serde_json::json!({
        "instruction": ExecutionInstruction::CollectOutput { handle: handle.clone() },
        "output": record.output,
    });

    Ok(AgentAction::Compound(vec![
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "execution_request".to_string(),
            event.clone(),
        )),
        AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
            summary: format!("collect {}", handle.0),
            state: "completed".to_string(),
            detail: event,
        }),
    ]))
}

fn handle_cancel_execution(handle: ExecutionHandle) -> Result<AgentAction, Error> {
    let mut store = execution_store()
        .lock()
        .map_err(|e| Error::Invariant(format!("execution store mutex poisoned: {e}")))?;
    let record = store.get_mut(&handle.0).ok_or_else(|| {
        Error::Sandbox(format!("execution handle '{}' is unknown", handle.0))
    })?;
    if matches!(record.status.state, ExecutionState::Queued | ExecutionState::Running) {
        record.status.state = ExecutionState::Cancelled;
        record.output.exit_code = Some(130);
    }
    let event = serde_json::json!({
        "instruction": ExecutionInstruction::CancelExecution { handle: handle.clone() },
        "status": record.status,
    });

    Ok(AgentAction::Compound(vec![
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "execution_request".to_string(),
            event.clone(),
        )),
        AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
            summary: format!("cancel {}", handle.0),
            state: format!("{:?}", record.status.state).to_ascii_lowercase(),
            detail: event,
        }),
    ]))
}

async fn run_allowlisted_tool(
    handle: ExecutionHandle,
    request: RunToolRequest,
    world: &dyn HStackWorld,
    memory: &WorkingMemory,
) -> ExecutionRecord {
    if request.tool_id != "light_compute" {
        let message = format!(
            "tool_id '{}' is not allowlisted in the public execution runtime",
            request.tool_id
        );
        return failed_record(
            handle,
            message,
        );
    }
    if !request.env_allowlist.is_empty() {
        return failed_record(
            handle,
            "light_compute execution does not accept env_allowlist entries".to_string(),
        );
    }
    if !matches!(request.network_policy, NetworkPolicy::Deny) {
        return failed_record(
            handle,
            "light_compute execution requires network_policy=deny".to_string(),
        );
    }
    if request.argv.is_empty() {
        return failed_record(
            handle,
            "light_compute execution requires argv[0] to contain source code".to_string(),
        );
    }

    let code = request.argv[0].clone();
    let input = match request.argv.get(1) {
        Some(raw) => match serde_json::from_str::<Value>(raw) {
            Ok(value) => value,
            Err(e) => {
                return failed_record(
                    handle,
                    format!("light_compute argv[1] must be valid JSON: {e}"),
                )
            }
        },
        None => serde_json::json!({}),
    };

    let tool = LightComputeTool::new();
    let args = serde_json::json!({
        "code": code,
        "input": input,
    });
    let action = match tool.execute(args, world, memory).await {
        Ok(action) => action,
        Err(error) => return failed_record(handle, error.to_string()),
    };
    let payload = match extract_light_compute_payload(action) {
        Ok(payload) => payload,
        Err(message) => return failed_record(handle, message),
    };

    let stdout = match serde_json::to_vec(&payload) {
        Ok(bytes) => bytes,
        Err(e) => {
            return failed_record(
                handle,
                format!("failed to serialize light_compute output: {e}"),
            )
        }
    };
    if (stdout.len() as u64) > request.limits.max_output_bytes {
        let message = format!(
            "light_compute output exceeds requested max_output_bytes {}",
            request.limits.max_output_bytes
        );
        return failed_record(
            handle,
            message,
        );
    }

    let ok = payload.get("ok").and_then(Value::as_bool) == Some(true);
    let state = if ok {
        ExecutionState::Completed
    } else {
        ExecutionState::Failed
    };
    let exit_code = Some(if ok { 0 } else { 1 });
    ExecutionRecord {
        status: ExecutionStatus {
            handle: handle.clone(),
            state,
            exit_code,
            stdout_bytes: stdout.len() as u64,
            stderr_bytes: 0,
        },
        output: ExecutionOutput {
            handle,
            stdout,
            stderr: Vec::new(),
            exit_code,
        },
    }
}

fn failed_record(
    handle: ExecutionHandle,
    message: String,
) -> ExecutionRecord {
    let stderr = message.into_bytes();
    let stderr_len = stderr.len() as u64;
    ExecutionRecord {
        status: ExecutionStatus {
            handle: handle.clone(),
            state: ExecutionState::Failed,
            exit_code: Some(1),
            stdout_bytes: 0,
            stderr_bytes: stderr_len,
        },
        output: ExecutionOutput {
            handle,
            stdout: Vec::new(),
            stderr,
            exit_code: Some(1),
        },
    }
}

fn extract_light_compute_payload(action: AgentAction) -> Result<Value, String> {
    match action {
        AgentAction::Compound(actions) => {
            for action in actions {
                if let AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) = action {
                    if key == "light_compute" {
                        return Ok(payload);
                    }
                }
            }
            Err("light_compute did not produce a technical payload".to_string())
        }
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
            if key == "light_compute" {
                Ok(payload)
            } else {
                Err("unexpected technical payload returned from light_compute".to_string())
            }
        }
        _ => Err("unexpected action returned from light_compute".to_string()),
    }
}

fn parse_run_tool_request(args: &Value) -> Result<RunToolRequest, Error> {
    let tool_id = args
        .get("tool_id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Provider("execution_request run_tool requires string 'tool_id'".to_string()))?
        .to_string();
    let argv = args
        .get("argv")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Provider("execution_request run_tool requires array 'argv'".to_string()))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| Error::Provider("execution_request 'argv' must contain strings".to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::Provider("execution_request run_tool requires string 'cwd'".to_string()))?;
    let cwd = VirtualPath::from_absolute(cwd)
        .map_err(|e| Error::Provider(format!("execution_request cwd error: {e}")))?;
    let env_allowlist = args
        .get("env_allowlist")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|value| {
                    value.as_str().map(str::to_string).ok_or_else(|| {
                        Error::Provider("execution_request 'env_allowlist' must contain strings".to_string())
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let stdin_policy = match args.get("stdin_policy").and_then(Value::as_str).unwrap_or("disabled") {
        "disabled" => StdinPolicy::Disabled,
        "empty" => StdinPolicy::Empty,
        _ => return Err(Error::Provider("execution_request stdin_policy must be 'disabled' or 'empty'".to_string())),
    };
    let network_policy = match args.get("network_policy").and_then(Value::as_str).unwrap_or("deny") {
        "deny" => NetworkPolicy::Deny,
        _ => return Err(Error::Provider("execution_request network_policy must be 'deny' in the public runtime".to_string())),
    };
    let output_streaming_policy = match args
        .get("output_streaming_policy")
        .and_then(Value::as_str)
        .unwrap_or("buffered")
    {
        "buffered" => OutputStreamingPolicy::Buffered,
        "streaming" => OutputStreamingPolicy::Streaming,
        _ => {
            return Err(Error::Provider(
                "execution_request output_streaming_policy must be 'buffered' or 'streaming'".to_string(),
            ))
        }
    };
    let filesystem_capability_profile = match args
        .get("filesystem_capability_profile")
        .and_then(Value::as_str)
        .unwrap_or("project_sandbox")
    {
        "safe_data_sandbox" => CapabilityProfile::SafeDataSandbox,
        "project_sandbox" => CapabilityProfile::ProjectSandbox,
        _ => {
            return Err(Error::Provider(
                "execution_request filesystem_capability_profile must be 'safe_data_sandbox' or 'project_sandbox'"
                    .to_string(),
            ))
        }
    };
    let timeout_ms = args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(1_000);
    let max_output_bytes = args
        .get("max_output_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(65_536);

    Ok(RunToolRequest {
        tool_id,
        argv,
        cwd,
        env_allowlist,
        stdin_policy,
        network_policy,
        output_streaming_policy,
        filesystem_capability_profile,
        limits: ExecutionLimits {
            timeout_ms,
            max_output_bytes,
        },
    })
}

fn parse_handle(args: &Value) -> Result<ExecutionHandle, Error> {
    args.get("handle")
        .and_then(Value::as_str)
        .map(|value| ExecutionHandle(value.to_string()))
        .ok_or_else(|| Error::Provider("execution_request requires string 'handle'".to_string()))
}