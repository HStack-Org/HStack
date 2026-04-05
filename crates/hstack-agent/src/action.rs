use hstack_core::provider::Message;
use hstack_core::sync::SyncAction;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::workspace::WorkspaceDelta;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DecodeAnomalyKind {
    AssistantContentIgnored,
    MultipleToolCallsInSingleTurn,
    ToolInvalidArguments { tool_name: String },
    ToolExecutionFailed { tool_name: String },
    UnknownTool { tool_name: String },
    NonActionableAssistantContent,
    NoActionableModelOutput,
    MultipleDecodeAnomalies,
}

/// A structured decode/runtime anomaly produced while interpreting a provider turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecodeAnomaly {
    pub kind: DecodeAnomalyKind,
    pub payload: Value,
}

impl DecodeAnomaly {
    pub fn assistant_content_ignored(content: String) -> Self {
        Self {
            kind: DecodeAnomalyKind::AssistantContentIgnored,
            payload: serde_json::json!({
                "reason": "content_with_tool_calls_has_no_semantic_effect",
                "content": content,
            }),
        }
    }

    pub fn multiple_tool_calls_in_single_turn(tool_names: Vec<String>) -> Self {
        Self {
            kind: DecodeAnomalyKind::MultipleToolCallsInSingleTurn,
            payload: serde_json::json!({
                "reason": "multiple_tool_calls_in_single_turn",
                "tool_names": tool_names,
            }),
        }
    }

    pub fn tool_invalid_arguments(tool_name: String, error: String, raw_arguments: String) -> Self {
        Self {
            kind: DecodeAnomalyKind::ToolInvalidArguments { tool_name },
            payload: serde_json::json!({
                "type": "invalid_arguments",
                "error": error,
                "raw_arguments": raw_arguments,
            }),
        }
    }

    pub fn tool_execution_failed(tool_name: String, error: String) -> Self {
        Self {
            kind: DecodeAnomalyKind::ToolExecutionFailed { tool_name },
            payload: serde_json::json!({
                "type": "tool_execution_failed",
                "error": error,
            }),
        }
    }

    pub fn unknown_tool(tool_name: String) -> Self {
        Self {
            kind: DecodeAnomalyKind::UnknownTool { tool_name },
            payload: serde_json::json!({
                "type": "unknown_tool",
                "error": "Unknown tool",
            }),
        }
    }

    pub fn non_actionable_assistant_content(content: String) -> Self {
        Self {
            kind: DecodeAnomalyKind::NonActionableAssistantContent,
            payload: serde_json::json!({
                "reason": "non_actionable_assistant_content",
                "content": content,
            }),
        }
    }

    pub fn no_actionable_model_output() -> Self {
        Self {
            kind: DecodeAnomalyKind::NoActionableModelOutput,
            payload: serde_json::json!({ "reason": "no_actionable_model_output" }),
        }
    }

    pub fn multiple(anomalies: Vec<DecodeAnomaly>) -> Self {
        Self {
            kind: DecodeAnomalyKind::MultipleDecodeAnomalies,
            payload: serde_json::json!({
                "reason": "multiple_decode_anomalies",
                "anomalies": anomalies.into_iter().map(|a| serde_json::json!({
                    "key": a.key(),
                    "payload": a.payload,
                })).collect::<Vec<_>>(),
            }),
        }
    }

    pub fn key(&self) -> String {
        match &self.kind {
            DecodeAnomalyKind::AssistantContentIgnored => "assistant_content_ignored".to_string(),
            DecodeAnomalyKind::MultipleToolCallsInSingleTurn => "agent_runtime".to_string(),
            DecodeAnomalyKind::ToolInvalidArguments { tool_name }
            | DecodeAnomalyKind::ToolExecutionFailed { tool_name }
            | DecodeAnomalyKind::UnknownTool { tool_name } => format!("tool_error:{tool_name}"),
            DecodeAnomalyKind::NonActionableAssistantContent
            | DecodeAnomalyKind::NoActionableModelOutput
            | DecodeAnomalyKind::MultipleDecodeAnomalies => "agent_runtime".to_string(),
        }
    }
}

/// The result of decoding a raw provider turn under the harness transition algebra.
///
/// A turn either yields a valid action to apply or a structural anomaly to record.
/// Raw provider output never has semantic effect on its own.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum DecodedTurn {
    Action(AgentAction),
    Anomaly(DecodeAnomaly),
}

/// Represents a modification to the agent's internal working memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkingMemoryDelta {
    /// Appends a message to the history.
    AppendMessage(Message),
    /// Injects technical context or raw tool output.
    AddTechnicalNoise(String, Value),
}

/// The "action" function `a` produced by the agent.
/// It represents the intent to transition the state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum AgentAction {
    /// Update short-term context.
    UpdateWorkingMemory(WorkingMemoryDelta),
    /// Update the typed workspace, including dock and apps.
    UpdateWorkspace(WorkspaceDelta),
    /// Propose changes to the long-term stack (requires safety control).
    /// Uses the canonical SyncAction from hstack-core.
    UpdateStack(SyncAction),
    /// A combination of multiple transitions.
    Compound(Vec<AgentAction>),
    /// Signal completion with a final answer.
    Stop(String),
}
