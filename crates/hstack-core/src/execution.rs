use crate::filesystem::CapabilityProfile;
use crate::virtual_fs::VirtualPath;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StdinPolicy {
    Disabled,
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkPolicy {
    Deny,
    AllowHostAllowlist(Vec<String>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputStreamingPolicy {
    Buffered,
    Streaming,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionLimits {
    pub timeout_ms: u64,
    pub max_output_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionHandle(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunToolRequest {
    pub tool_id: String,
    pub argv: Vec<String>,
    pub cwd: VirtualPath,
    pub env_allowlist: Vec<String>,
    pub stdin_policy: StdinPolicy,
    pub network_policy: NetworkPolicy,
    pub output_streaming_policy: OutputStreamingPolicy,
    pub filesystem_capability_profile: CapabilityProfile,
    pub limits: ExecutionLimits,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionInstruction {
    RunTool(RunToolRequest),
    PollExecution {
        handle: ExecutionHandle,
    },
    CancelExecution {
        handle: ExecutionHandle,
    },
    CollectOutput {
        handle: ExecutionHandle,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExecutionState {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionStatus {
    pub handle: ExecutionHandle,
    pub state: ExecutionState,
    pub exit_code: Option<i32>,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionOutput {
    pub handle: ExecutionHandle,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: Option<i32>,
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionInstruction, ExecutionLimits, NetworkPolicy, OutputStreamingPolicy, RunToolRequest,
        StdinPolicy,
    };
    use crate::filesystem::CapabilityProfile;
    use crate::virtual_fs::VirtualPath;

    #[test]
    fn run_tool_request_serializes_cwd_and_profile() {
        let request = RunToolRequest {
            tool_id: "python_sandbox".to_string(),
            argv: vec!["script.py".to_string()],
            cwd: VirtualPath::from_absolute("/workspace")
                .unwrap_or_else(|e| panic!("cwd parse failed: {e}")),
            env_allowlist: vec!["LANG".to_string()],
            stdin_policy: StdinPolicy::Disabled,
            network_policy: NetworkPolicy::Deny,
            output_streaming_policy: OutputStreamingPolicy::Buffered,
            filesystem_capability_profile: CapabilityProfile::ProjectSandbox,
            limits: ExecutionLimits {
                timeout_ms: 1_000,
                max_output_bytes: 65_536,
            },
        };

        let json = serde_json::to_string(&ExecutionInstruction::RunTool(request))
            .unwrap_or_else(|e| panic!("execution instruction serialization failed: {e}"));
        assert!(json.contains("python_sandbox"));
        assert!(json.contains("/workspace"));
        assert!(json.contains("ProjectSandbox"));
    }
}