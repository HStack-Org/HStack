use async_trait::async_trait;
use hstack_core::filesystem::{CapabilityProfile, FilesystemInstruction, FilesystemPolicy};
use hstack_core::virtual_fs::VirtualPath;
use serde_json::Value;
use std::path::PathBuf;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::filesystem::{configured_local_filesystem, LocalSandboxedFilesystem};
use crate::tool::microbash::workspace_actions_for_instruction;
use crate::tool::Tool;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::workspace::WorkspaceDelta;

pub struct FilesystemPatchTool {
    sandbox_root_override: Option<PathBuf>,
    capability_profile: CapabilityProfile,
}

impl FilesystemPatchTool {
    pub fn new() -> Self {
        Self {
            sandbox_root_override: None,
            capability_profile: CapabilityProfile::ProjectSandbox,
        }
    }

    pub fn new_with_root(root: PathBuf) -> Self {
        Self {
            sandbox_root_override: Some(root),
            capability_profile: CapabilityProfile::ProjectSandbox,
        }
    }

    #[cfg(test)]
    pub fn new_for_tests(root: PathBuf) -> Self {
        Self::new_with_root(root)
    }

    fn backend(&self) -> Result<LocalSandboxedFilesystem, Error> {
        if let Some(root) = &self.sandbox_root_override {
            let policy = match self.capability_profile {
                CapabilityProfile::SafeDataSandbox => FilesystemPolicy::safe_data_sandbox(VirtualPath::root()),
                CapabilityProfile::ProjectSandbox => FilesystemPolicy::project_sandbox(VirtualPath::root()),
            };
            return LocalSandboxedFilesystem::new(root.clone(), policy);
        }

        configured_local_filesystem()
    }
}

impl Default for FilesystemPatchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for FilesystemPatchTool {
    fn name(&self) -> &str {
        "filesystem_patch"
    }

    fn description(&self) -> &str {
        "Applies a structured row-span edit to a sandboxed file and updates the editor and file tree apps."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": { "type": "string", "enum": ["append", "replace", "insert", "delete"] },
                "path": { "type": "string" },
                "row_start": { "type": "integer", "minimum": 0 },
                "row_end": { "type": "integer", "minimum": 0, "description": "Exclusive end row for replace/delete spans." },
                "replacement_text": { "type": "string", "description": "Replacement text block. Embedded newlines are allowed." },
                "expected_conflict_token": { "type": "string" }
            },
            "required": ["operation", "path"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("filesystem_patch requires an 'operation' string".to_string()))?;
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("filesystem_patch requires a string 'path'".to_string()))?;
        let path = VirtualPath::from_absolute(path)
            .map_err(|e| Error::Provider(format!("filesystem_patch path error: {e}")))?;
        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                return Ok(build_patch_failure_action(
                    &path,
                    "configuration",
                    error.to_string(),
                ))
            }
        };
        let existing = match backend.read_file(&path, 0, 1024 * 1024) {
            Ok(existing) => existing,
            Err(error) => {
                return Ok(build_patch_failure_action(
                    &path,
                    "backend",
                    error.to_string(),
                ))
            }
        };
        let existing_content = match String::from_utf8(existing.content.clone()) {
            Ok(content) => content,
            Err(_) => {
                return Ok(build_patch_failure_action(
                    &path,
                    "backend",
                    format!("file '{}' is not valid UTF-8", path),
                ))
            }
        };
        let existing_line_count = append_line_index(&existing_content);
        let row_start = parse_row_start(&args, operation, existing_line_count)?;
        let row_end = parse_row_end(&args, operation, row_start)?;
        if row_end < row_start {
            return Err(Error::Provider("filesystem_patch requires row_end >= row_start".to_string()));
        }
        let new_lines = parse_replacement_lines(&args, operation)?;
        let delete_count = match operation {
            "append" | "insert" => 0,
            "replace" | "delete" => row_end - row_start,
            _ => return Err(Error::Provider("filesystem_patch received unsupported operation".to_string())),
        };
        let expected_conflict_token = args
            .get("expected_conflict_token")
            .and_then(Value::as_str)
            .map(str::to_string);
        let patch_lines = if operation == "delete" { Vec::new() } else { new_lines };
        let patch = serde_json::json!({
            "start_line": row_start,
            "delete_count": delete_count,
            "new_lines": patch_lines,
        })
        .to_string();

        let instruction = FilesystemInstruction::PatchFile {
            path: path.clone(),
            patch,
            expected_conflict_token: expected_conflict_token.map(hstack_core::filesystem::ConflictToken),
        };
        let outcome = match backend.execute(&instruction) {
            Ok(outcome) => outcome,
            Err(error) => {
                return Ok(build_patch_failure_action(
                    &path,
                    "backend",
                    error.to_string(),
                ))
            }
        };

        let event = serde_json::json!({
            "ok": true,
            "instruction": instruction,
            "outcome": outcome,
        });

        let mut actions = vec![
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
                "filesystem_patch".to_string(),
                event.clone(),
            )),
            AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
                summary: format!("filesystem_patch {}", path),
                state: "ok".to_string(),
                detail: event,
            }),
        ];
        for delta in workspace_actions_for_instruction(&backend, &instruction, &outcome)? {
            actions.push(AgentAction::UpdateWorkspace(delta));
        }
        Ok(AgentAction::Compound(actions))
    }
}

fn parse_row_start(args: &Value, operation: &str, append_row: usize) -> Result<usize, Error> {
    match operation {
        "append" => Ok(append_row),
        "replace" | "insert" | "delete" => args
            .get("row_start")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| Error::Provider("filesystem_patch requires integer 'row_start'".to_string())),
        _ => Err(Error::Provider("filesystem_patch received unsupported operation".to_string())),
    }
}

fn parse_row_end(args: &Value, operation: &str, row_start: usize) -> Result<usize, Error> {
    match operation {
        "append" | "insert" => Ok(row_start),
        "replace" | "delete" => args
            .get("row_end")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| Error::Provider("filesystem_patch requires integer 'row_end' for replace and delete".to_string())),
        _ => Err(Error::Provider("filesystem_patch received unsupported operation".to_string())),
    }
}

fn parse_replacement_lines(args: &Value, operation: &str) -> Result<Vec<String>, Error> {
    let Some(replacement_text) = args.get("replacement_text") else {
        return match operation {
            "append" | "replace" | "insert" => Err(Error::Provider(
                "filesystem_patch requires string 'replacement_text' for append, replace, and insert"
                    .to_string(),
            )),
            "delete" => Ok(Vec::new()),
            _ => Err(Error::Provider("filesystem_patch received unsupported operation".to_string())),
        };
    };

    let replacement_text = replacement_text
        .as_str()
        .ok_or_else(|| Error::Provider("filesystem_patch 'replacement_text' must be a string".to_string()))?;
    Ok(split_replacement_text(replacement_text))
}

fn split_replacement_text(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if text.ends_with('\n') {
        let _ = lines.pop();
    }
    lines
}

fn append_line_index(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }

    content.lines().count()
}

fn build_patch_failure_action(path: &VirtualPath, kind: &str, message: String) -> AgentAction {
    let event = serde_json::json!({
        "ok": false,
        "path": path,
        "error": {
            "type": kind,
            "message": message,
        }
    });

    AgentAction::Compound(vec![
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "filesystem_patch".to_string(),
            event.clone(),
        )),
        AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
            summary: format!("filesystem_patch {}", path),
            state: "error".to_string(),
            detail: event,
        }),
    ])
}