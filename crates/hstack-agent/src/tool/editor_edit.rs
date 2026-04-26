use async_trait::async_trait;
use hstack_core::filesystem::{CapabilityProfile, ConflictToken, FilesystemInstruction, FilesystemPolicy};
use hstack_core::virtual_fs::VirtualPath;
use serde_json::Value;
use std::path::PathBuf;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::filesystem::{configured_local_filesystem, LocalSandboxedFilesystem};
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::microbash::workspace_actions_for_instruction;
use crate::tool::Tool;
use crate::workspace::{AppId, WorkspaceDelta};

pub struct EditorEditTool {
    sandbox_root_override: Option<PathBuf>,
    capability_profile: CapabilityProfile,
}

impl EditorEditTool {
    pub fn new() -> Self {
        Self {
            sandbox_root_override: None,
            capability_profile: CapabilityProfile::ProjectSandbox,
        }
    }

    #[cfg(test)]
    pub fn new_for_tests(root: PathBuf) -> Self {
        Self {
            sandbox_root_override: Some(root),
            capability_profile: CapabilityProfile::ProjectSandbox,
        }
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

impl Default for EditorEditTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for EditorEditTool {
    fn name(&self) -> &str {
        "editor_edit"
    }

    fn description(&self) -> &str {
        "Edits the open editor buffer or an explicit virtual file path by replacing a row span through the secured virtual filesystem backend."
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
            "required": ["operation"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let operation = args
            .get("operation")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Provider("editor_edit requires an 'operation' string".to_string()))?;
        let new_lines = parse_replacement_lines(&args, operation)?;

        let path = resolve_target_path(args.get("path"), memory)?;
        let backend = self.backend()?;
        let existing = backend.read_file(&path, 0, 1024 * 1024)?;
        let existing_content = String::from_utf8(existing.content.clone()).map_err(|_| {
            Error::Sandbox(format!("file '{}' is not valid UTF-8", path))
        })?;
        let existing_line_count = append_line_index(&existing_content);

        let row_start = parse_row_start(&args, operation, existing_line_count)?;
        let row_end = parse_row_end(&args, operation, row_start)?;
        if row_end < row_start {
            return Err(Error::Provider("editor_edit requires row_end >= row_start".to_string()));
        }
        let delete_count = match operation {
            "append" | "insert" => 0,
            "replace" | "delete" => row_end - row_start,
            _ => return Err(Error::Provider("editor_edit received unsupported operation".to_string())),
        };
        let expected_conflict_token = resolve_expected_conflict_token(args.get("expected_conflict_token"), memory, &path, existing.conflict_token.clone())?;
        let patch_lines = if operation == "delete" { Vec::new() } else { new_lines };

        let instruction = FilesystemInstruction::PatchFile {
            path: path.clone(),
            patch: serde_json::json!({
                "start_line": row_start,
                "delete_count": delete_count,
                "new_lines": patch_lines,
            })
            .to_string(),
            expected_conflict_token,
        };

        let outcome = backend.execute(&instruction)?;
        let event = serde_json::json!({
            "ok": true,
            "instruction": instruction,
            "outcome": outcome,
        });

        let mut actions = vec![
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
                "editor_edit".to_string(),
                event.clone(),
            )),
            AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
                summary: format!("editor_edit {}", path),
                state: "ok".to_string(),
                detail: event,
            }),
        ];
        for delta in workspace_actions_for_instruction(&backend, &instruction, &outcome)? {
            actions.push(AgentAction::UpdateWorkspace(delta));
        }
        actions.push(AgentAction::UpdateWorkspace(WorkspaceDelta::FocusApp(AppId::Editor)));
        Ok(AgentAction::Compound(actions))
    }
}

fn resolve_target_path(path_value: Option<&Value>, memory: &WorkingMemory) -> Result<VirtualPath, Error> {
    if let Some(path) = path_value.and_then(Value::as_str) {
        return VirtualPath::from_absolute(path)
            .map_err(|e| Error::Provider(format!("editor_edit path error: {e}")));
    }

    memory
        .workspace
        .editor
        .buffer
        .as_ref()
        .map(|buffer| buffer.path.clone())
        .ok_or_else(|| Error::Provider("editor_edit requires 'path' when no editor buffer is open".to_string()))
}

fn resolve_expected_conflict_token(
    token_value: Option<&Value>,
    memory: &WorkingMemory,
    path: &VirtualPath,
    fallback: Option<ConflictToken>,
) -> Result<Option<ConflictToken>, Error> {
    if let Some(token) = token_value {
        return token
            .as_str()
            .map(|value| Some(ConflictToken(value.to_string())))
            .ok_or_else(|| Error::Provider("editor_edit 'expected_conflict_token' must be a string".to_string()));
    }

    Ok(
        memory
            .workspace
            .editor
            .buffer
            .as_ref()
            .filter(|buffer| &buffer.path == path)
            .and_then(|buffer| buffer.conflict_token.clone())
            .or(fallback),
    )
}

fn parse_row_start(args: &Value, operation: &str, append_row: usize) -> Result<usize, Error> {
    match operation {
        "append" => Ok(append_row),
        "replace" | "insert" | "delete" => args
            .get("row_start")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| Error::Provider("editor_edit requires integer 'row_start'".to_string())),
        _ => Err(Error::Provider("editor_edit received unsupported operation".to_string())),
    }
}

fn parse_row_end(args: &Value, operation: &str, row_start: usize) -> Result<usize, Error> {
    match operation {
        "append" | "insert" => Ok(row_start),
        "replace" | "delete" => args
            .get("row_end")
            .and_then(Value::as_u64)
            .map(|value| value as usize)
            .ok_or_else(|| Error::Provider("editor_edit requires integer 'row_end' for replace and delete".to_string())),
        _ => Err(Error::Provider("editor_edit received unsupported operation".to_string())),
    }
}

fn append_line_index(content: &str) -> usize {
    if content.is_empty() {
        return 0;
    }

    let line_count = content.lines().count();
    if content.ends_with('\n') {
        line_count
    } else {
        line_count
    }
}

fn parse_replacement_lines(args: &Value, operation: &str) -> Result<Vec<String>, Error> {
    let Some(replacement_text) = args.get("replacement_text") else {
        return match operation {
            "append" | "replace" | "insert" => Err(Error::Provider(
                "editor_edit requires string 'replacement_text' for append, replace, and insert"
                    .to_string(),
            )),
            "delete" => Ok(Vec::new()),
            _ => Err(Error::Provider("editor_edit received unsupported operation".to_string())),
        };
    };

    let replacement_text = replacement_text
        .as_str()
        .ok_or_else(|| Error::Provider("editor_edit 'replacement_text' must be a string".to_string()))?;
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