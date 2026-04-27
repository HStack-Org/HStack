use async_trait::async_trait;
use hstack_core::filesystem::{CapabilityProfile, FilesystemInstruction, FilesystemOutcome, FilesystemPolicy};
use hstack_core::virtual_fs::VirtualPath;
use serde_json::Value;
use std::path::PathBuf;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::filesystem::{configured_local_filesystem, LocalSandboxedFilesystem};
use crate::memory::{HStackWorld, WorkingMemory};
use crate::microbash::{parse_and_lower, MicrobashError};
use crate::tool::Tool;
use crate::workspace::WorkspaceDelta;

pub struct MicrobashTool {
    sandbox_root_override: Option<PathBuf>,
    capability_profile: CapabilityProfile,
}

impl MicrobashTool {
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

impl Default for MicrobashTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for MicrobashTool {
    fn name(&self) -> &str {
        "microbash"
    }

    fn description(&self) -> &str {
        "Runs constrained microbash commands against the configured sandboxed workspace and updates file tree, editor, search, and job apps."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string" }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .ok_or_else(|| Error::Provider("microbash requires a non-empty 'command' string".to_string()))?;

        let cwd = memory.workspace.filesystem_cwd.clone();
        let instructions = match parse_and_lower(&cwd, command) {
            Ok(instructions) => instructions,
            Err(error) => return Ok(build_failure_action(command, &cwd, failure_type_for_parse(&error), error.to_string())),
        };

        let backend = match self.backend() {
            Ok(backend) => backend,
            Err(error) => {
                return Ok(build_failure_action(
                    command,
                    &cwd,
                    "configuration",
                    error.to_string(),
                ))
            }
        };

        let mut outcomes = Vec::new();
        let mut workspace_actions = Vec::new();
        for instruction in &instructions {
            let outcome = match backend.execute(instruction) {
                Ok(outcome) => outcome,
                Err(error) => {
                    return Ok(build_failure_action(
                        command,
                        &cwd,
                        "backend",
                        error.to_string(),
                    ))
                }
            };

            workspace_actions.extend(workspace_actions_for_instruction(&backend, instruction, &outcome)?);
            outcomes.push(outcome);
        }

        let event = serde_json::json!({
            "ok": true,
            "command": command,
            "cwd": cwd,
            "instructions": instructions,
            "outcomes": outcomes,
        });

        let mut actions = vec![
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
                "microbash".to_string(),
                event.clone(),
            )),
            AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
                summary: format!("microbash {command}"),
                state: "ok".to_string(),
                detail: event,
            }),
        ];
        actions.extend(workspace_actions.into_iter().map(AgentAction::UpdateWorkspace));
        Ok(AgentAction::Compound(actions))
    }
}

pub(crate) fn workspace_actions_for_instruction(
    backend: &LocalSandboxedFilesystem,
    instruction: &FilesystemInstruction,
    outcome: &FilesystemOutcome,
) -> Result<Vec<WorkspaceDelta>, Error> {
    let mut actions = Vec::new();
    match (instruction, outcome) {
        (FilesystemInstruction::ListDir { path, .. }, FilesystemOutcome::ListDir(entries)) => {
            actions.push(WorkspaceDelta::PublishFilesystemTree {
                cwd: path.clone(),
                entries: entries.clone(),
            });
        }
        (FilesystemInstruction::ReadFile { .. }, FilesystemOutcome::ReadFile(result)) => {
            let content = String::from_utf8(result.content.clone())
                .map_err(|_| Error::Sandbox(format!("file '{}' is not valid UTF-8", result.path)))?;
            actions.push(WorkspaceDelta::PublishEditorBuffer {
                path: result.path.clone(),
                conflict_token: result.conflict_token.clone(),
                content,
            });
        }
        (FilesystemInstruction::SearchText { scope, query, .. }, FilesystemOutcome::SearchText(matches)) => {
            actions.push(WorkspaceDelta::PublishFilesystemSearch {
                query: query.clone(),
                scope_root: scope.root.clone(),
                matches: matches.clone(),
            });
        }
        (FilesystemInstruction::WriteFile { path, content, .. }, FilesystemOutcome::WriteFile(result)) => {
            let text = String::from_utf8(content.clone())
                .map_err(|_| Error::Sandbox(format!("written file '{}' is not valid UTF-8", path)))?;
            actions.push(WorkspaceDelta::PublishEditorBuffer {
                path: path.clone(),
                conflict_token: result.conflict_token.clone(),
                content: text,
            });
            if let Some(tree) = refresh_parent_listing(backend, path)? {
                actions.push(tree);
            }
        }
        (FilesystemInstruction::CreateDir { path, .. }, FilesystemOutcome::CreateDir) => {
            if let Some(tree) = refresh_parent_listing(backend, path)? {
                actions.push(tree);
            }
        }
        (FilesystemInstruction::MovePath { from, to, .. }, FilesystemOutcome::MovePath(_)) => {
            if let Some(tree) = refresh_parent_listing(backend, from)? {
                actions.push(tree);
            }
            if let Some(tree) = refresh_parent_listing(backend, to)? {
                actions.push(tree);
            }
        }
        (FilesystemInstruction::DeletePath { path, .. }, FilesystemOutcome::DeletePath(_)) => {
            if let Some(tree) = refresh_parent_listing(backend, path)? {
                actions.push(tree);
            }
        }
        (FilesystemInstruction::PatchFile { path, .. }, FilesystemOutcome::PatchFile(result)) => {
            let read = backend.read_file(path, 0, u64::MAX.min(1024 * 1024))?;
            let content = String::from_utf8(read.content)
                .map_err(|_| Error::Sandbox(format!("file '{}' is not valid UTF-8", path)))?;
            actions.push(WorkspaceDelta::PublishEditorBuffer {
                path: path.clone(),
                conflict_token: result.conflict_token.clone(),
                content,
            });
            if let Some(tree) = refresh_parent_listing(backend, path)? {
                actions.push(tree);
            }
        }
        _ => {}
    }
    Ok(actions)
}

fn refresh_parent_listing(
    backend: &LocalSandboxedFilesystem,
    path: &VirtualPath,
) -> Result<Option<WorkspaceDelta>, Error> {
    let Some(parent) = path.parent() else {
        return Ok(None);
    };

    if let Ok(entries) = backend.list_dir(&parent, None) {
        return Ok(Some(WorkspaceDelta::PublishFilesystemTree { cwd: parent, entries }));
    }
    Ok(None)
}

fn build_failure_action(command: &str, cwd: &VirtualPath, kind: &str, message: String) -> AgentAction {
    let event = serde_json::json!({
        "ok": false,
        "command": command,
        "cwd": cwd,
        "error": {
            "type": kind,
            "message": message,
        }
    });

    AgentAction::Compound(vec![
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            "microbash".to_string(),
            event.clone(),
        )),
        AgentAction::UpdateWorkspace(WorkspaceDelta::RecordJob {
            summary: format!("microbash {command}"),
            state: "error".to_string(),
            detail: event,
        }),
    ])
}

fn failure_type_for_parse(error: &MicrobashError) -> &'static str {
    match error {
        MicrobashError::Parse(_) => "parse",
        MicrobashError::UnsupportedConstruct(_) | MicrobashError::UnsupportedCommand(_) => "unsupported",
        MicrobashError::InvalidOption(_) | MicrobashError::MissingArgument(_) => "invalid_arguments",
        MicrobashError::InvalidPath(_) => "path_policy",
    }
}