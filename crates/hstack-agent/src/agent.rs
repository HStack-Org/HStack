use crate::memory::{HStackWorld, WorkingMemory};
use crate::manager::ContextManager;
use crate::control::AgentControlSystem;
use crate::tool::Tool;
use crate::action::{AgentAction, DecodeAnomaly, DecodedTurn, WorkingMemoryDelta};
use crate::provider::{LlmProvider, Message, Role};
use crate::error::Error;
use hstack_core::sync::SyncAction;
use serde_json::Value;
use tracing::{debug, info, warn};
use futures::future::BoxFuture;

const HOST_TERMINAL_FALLBACK_ANSWER: &str = "I could not complete a valid tool-grounded response for this turn.";

/// The central orchestrator of the agentic harness.
/// Implements an action-based transition loop over the current context state.
///
/// Raw provider output is not itself a state transition. A model turn only has
/// semantic effect after it is decoded into a valid `AgentAction` and applied.
/// In particular, bare assistant prose is not a terminal answer unless it is
/// carried by a terminal action such as `AgentAction::Stop`.
pub struct Agent {
    pub provider: Box<dyn LlmProvider>,
    pub manager: Box<dyn ContextManager>,
    pub control: Box<dyn AgentControlSystem>,
    pub tools: Vec<Box<dyn Tool>>,
    pub base_prompt: String,
}

#[derive(Clone)]
pub struct AgentProgressUpdate {
    pub iteration: usize,
    pub phase: String,
    pub working_memory: WorkingMemory,
}

impl Agent {
    /// Runs the agentic loop until completion or max depth.
    /// Returns the final answer string and a list of validated SyncActions (the "Delta List").
    pub async fn run(
        &self,
        world: &dyn HStackWorld,
        memory: &mut WorkingMemory,
    ) -> Result<(String, Vec<SyncAction>), Error> {
        self.run_with_progress(world, memory, |_| {}).await
    }

    /// Runs the agentic loop and emits progress updates for live debugging UIs.
    pub async fn run_with_progress<F>(
        &self,
        world: &dyn HStackWorld,
        memory: &mut WorkingMemory,
        mut on_progress: F,
    ) -> Result<(String, Vec<SyncAction>), Error>
    where
        F: FnMut(AgentProgressUpdate),
    {
        let max_iterations = 10;
        let mut iterations = 0;
        let mut collected_deltas = Vec::new();

        loop {
            on_progress(AgentProgressUpdate {
                iteration: iterations,
                phase: "iteration_start".to_string(),
                working_memory: memory.clone(),
            });

            if iterations >= max_iterations {
                memory.technical_noise.push(serde_json::json!({
                    "agent_limit": {
                        "reason": "max_iterations",
                        "max_iterations": max_iterations,
                        "iteration": iterations,
                    }
                }));
                on_progress(AgentProgressUpdate {
                    iteration: iterations,
                    phase: "limit_reached".to_string(),
                    working_memory: memory.clone(),
                });
                return self
                    .enforce_identity_response(
                        world,
                        memory,
                        &mut collected_deltas,
                        "forced_terminal_turn",
                        "max_iterations",
                    )
                    .await;
            }

            info!(iteration = iterations, "Starting agent reasoning step");

            // 1. Construct context (C_n)
            let stack_snapshot = world.get_stack_snapshot().await.map_err(Error::World)?;
            let tickets = stack_snapshot.projected_agent_tickets(&memory.proposed_stack_actions);
            memory.workspace.refresh_near_events(&tickets);
            let _ = memory.workspace.materialize_allocation_plan();
            let tool_schemas = build_tool_schemas(&self.tools);
            let mut messages = self.manager.construct_context(world, memory, &self.base_prompt).await?;
            insert_tool_contract_message(&mut messages, &self.tools, false);

            // 2. Generate response from provider
            let response = self.provider.generate_content(&messages, Some(&tool_schemas)).await?;

            memory.technical_noise.push(serde_json::json!({
                "agent_iteration": {
                    "index": iterations,
                    "assistant_content": response.content.clone(),
                    "tool_calls": response.tool_calls.as_ref().map(|calls| {
                        calls
                            .iter()
                            .map(|c| serde_json::json!({
                                "name": c.function.name,
                                "arguments": c.function.arguments,
                            }))
                            .collect::<Vec<_>>()
                    }).unwrap_or_default(),
                }
            }));

            // 4. Resolve the response into actions
            let decoded_turn = self
                .resolve_response_to_action(response.clone(), world, memory, &self.tools)
                .await?;

            // 5. Apply the transition and collect deltas
            match decoded_turn {
                DecodedTurn::Action(action) => {
                    match self.apply_action(action, world, memory, &mut collected_deltas).await? {
                        Some(final_answer) => {
                            info!(deltas = collected_deltas.len(), "Agent reached terminal state");
                            on_progress(AgentProgressUpdate {
                                iteration: iterations,
                                phase: "completed".to_string(),
                                working_memory: memory.clone(),
                            });
                            return Ok((final_answer, collected_deltas));
                        }
                        None => {
                            on_progress(AgentProgressUpdate {
                                iteration: iterations,
                                phase: "iteration_applied".to_string(),
                                working_memory: memory.clone(),
                            });
                            iterations += 1;
                        }
                    }
                }
                DecodedTurn::Anomaly(anomaly) => {
                    let trace_reason = anomaly_trace_reason(&anomaly).to_string();
                    self.record_decode_anomaly(memory, anomaly);
                    on_progress(AgentProgressUpdate {
                        iteration: iterations,
                        phase: "identity_enforcement".to_string(),
                        working_memory: memory.clone(),
                    });
                    return self
                        .enforce_identity_response(
                            world,
                            memory,
                            &mut collected_deltas,
                            "identity_enforcement_turn",
                            &trace_reason,
                        )
                        .await;
                }
            }
        }
    }

    /// Decodes raw provider output into the transition algebra used by the harness.
    ///
    /// This function should remain strict: only valid tool-driven actions or
    /// explicit runtime anomalies belong here. It must not treat free-form
    /// assistant prose as completion by itself.
    async fn resolve_response_to_action(
        &self,
        response: Message,
        world: &dyn HStackWorld,
        memory: &WorkingMemory,
        allowed_tools: &[Box<dyn Tool>],
    ) -> Result<DecodedTurn, Error> {
        // HEURISITC STRICLY FORBIDDEN.
        // Runtime decode may reject only structural
        // protocol invalidity. It must not smuggle in history-based blockers such as
        // "this tool call feels repetitive" because that changes the math.
        let mut actions = Vec::new();
        let mut anomalies = Vec::new();
        let assistant_content = response
            .content
            .as_ref()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());

        let tool_calls = response.tool_calls.clone().unwrap_or_default();

        if !tool_calls.is_empty() {
            actions.push(AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AppendMessage(
                response.clone(),
            )));

            if let Some(content) = assistant_content.as_ref() {
                anomalies.push(DecodeAnomaly::assistant_content_ignored(content.clone()));
            }
            if tool_calls.len() > 1 {
                return Ok(DecodedTurn::Anomaly(
                    DecodeAnomaly::multiple_tool_calls_in_single_turn(
                        tool_calls
                            .into_iter()
                            .map(|call| call.function.name)
                            .collect(),
                    ),
                ));
            }
            for call in tool_calls {
                let tool = allowed_tools.iter().find(|t| t.name() == call.function.name);
                match tool {
                    Some(t) => {
                        let args = match serde_json::from_str::<serde_json::Value>(&call.function.arguments) {
                            Ok(args) => args,
                            Err(err) => {
                                warn!(tool = %t.name(), error = %err, "Tool arguments were malformed");
                                anomalies.push(DecodeAnomaly::tool_invalid_arguments(
                                    t.name().to_string(),
                                    err.to_string(),
                                    call.function.arguments,
                                ));
                                actions.push(AgentAction::UpdateWorkingMemory(
                                    WorkingMemoryDelta::AppendMessage(crate::provider::Message {
                                        role: crate::provider::Role::Tool,
                                        content: Some(format!("tool {} failed: invalid arguments", t.name())),
                                        tool_calls: None,
                                        tool_call_id: Some(call.id.clone()),
                                        name: Some(t.name().to_string()),
                                    })
                                ));
                                continue;
                            }
                        };
                        let tool_name = t.name();
                        actions.push(AgentAction::UpdateWorkingMemory(
                            WorkingMemoryDelta::AddTechnicalNoise(
                                format!("tool_call:{tool_name}"),
                                serde_json::json!({ "arguments": args.clone() }),
                            ),
                        ));
                        debug!(tool = %t.name(), "Executing tool");
                        match t.execute(args, world, memory).await {
                            Ok(tool_action) => {
                                actions.push(tool_action);
                                actions.push(AgentAction::UpdateWorkingMemory(
                                    WorkingMemoryDelta::AppendMessage(crate::provider::Message {
                                        role: crate::provider::Role::Tool,
                                        content: Some(format!("tool {} executed successfully", t.name())),
                                        tool_calls: None,
                                        tool_call_id: Some(call.id.clone()),
                                        name: Some(t.name().to_string()),
                                    })
                                ));
                            }
                            Err(err) => {
                                warn!(tool = %t.name(), error = %err, "Tool execution failed; continuing with fallback trace");
                                anomalies.push(DecodeAnomaly::tool_execution_failed(
                                    t.name().to_string(),
                                    err.to_string(),
                                ));
                                actions.push(AgentAction::UpdateWorkingMemory(
                                    WorkingMemoryDelta::AppendMessage(crate::provider::Message {
                                        role: crate::provider::Role::Tool,
                                        content: Some(format!("tool {} failed: {}", t.name(), err)),
                                        tool_calls: None,
                                        tool_call_id: Some(call.id.clone()),
                                        name: Some(t.name().to_string()),
                                    })
                                ));
                            }
                        }
                    }
                    None => {
                        warn!(tool = %call.function.name, "LLM requested unknown tool");
                        anomalies.push(DecodeAnomaly::unknown_tool(call.function.name.clone()));
                        actions.push(AgentAction::UpdateWorkingMemory(
                            WorkingMemoryDelta::AppendMessage(crate::provider::Message {
                                role: crate::provider::Role::Tool,
                                content: Some(format!("tool {} is unknown", call.function.name)),
                                tool_calls: None,
                                tool_call_id: Some(call.id.clone()),
                                name: Some(call.function.name),
                            })
                        ));
                    }
                }
            }
        }

        if let Some(content) = assistant_content {
            if actions.is_empty() {
                anomalies.push(DecodeAnomaly::non_actionable_assistant_content(content));
            }
        }

        if actions.is_empty() {
            if anomalies.is_empty() {
                Ok(DecodedTurn::Anomaly(DecodeAnomaly::no_actionable_model_output()))
            } else if anomalies.len() == 1 {
                Ok(DecodedTurn::Anomaly(anomalies.remove(0)))
            } else {
                Ok(DecodedTurn::Anomaly(DecodeAnomaly::multiple(anomalies)))
            }
        } else {
            let mut leading_noise_actions: Vec<AgentAction> = anomalies
                .into_iter()
                .map(Self::anomaly_to_action)
                .collect();
            leading_noise_actions.append(&mut actions);

            if leading_noise_actions.len() == 1 {
                Ok(DecodedTurn::Action(leading_noise_actions.remove(0)))
            } else {
                Ok(DecodedTurn::Action(AgentAction::Compound(leading_noise_actions)))
            }
        }
    }

    fn anomaly_to_action(anomaly: DecodeAnomaly) -> AgentAction {
        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(
            anomaly.key(),
            anomaly.payload,
        ))
    }

    fn record_decode_anomaly(&self, memory: &mut WorkingMemory, anomaly: DecodeAnomaly) {
        memory
            .technical_noise
            .push(serde_json::json!({ anomaly.key(): anomaly.payload }));
    }

    async fn enforce_identity_response(
        &self,
        world: &dyn HStackWorld,
        memory: &mut WorkingMemory,
        collected_deltas: &mut Vec<SyncAction>,
        trace_key: &str,
        trace_reason: &str,
    ) -> Result<(String, Vec<SyncAction>), Error> {
        let forced_prompt = "MANDATORY TERMINATION\n- You must now terminate immediately.\n- The only valid terminal action is the identity tool.\n- Call identity exactly once now.\n- If you cannot provide natural-language content, use identity with an empty answer string.\n- No other tool exists in this turn.";

        let mut messages = self
            .manager
            .construct_context(world, memory, forced_prompt)
            .await?;

        let identity_tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == "identity")
            .ok_or_else(|| Error::Invariant("identity tool is missing from agent configuration".to_string()))?;

        let allowed_tools: Vec<&dyn Tool> = vec![identity_tool.as_ref()];
        insert_tool_contract_message_for_refs(&mut messages, &allowed_tools, true);

        let tool_schemas = vec![hstack_core::provider::Tool {
            r#type: "function".to_string(),
            function: hstack_core::provider::ToolFunction {
                name: identity_tool.name().to_string(),
                description: identity_tool.description().to_string(),
                parameters: identity_tool.parameters(),
            },
        }];

        let response = self.provider.generate_content(&messages, Some(&tool_schemas)).await?;
        memory.technical_noise.push(serde_json::json!({
            trace_key: {
                "reason": trace_reason,
                "assistant_content": response.content.clone(),
                "tool_calls": response.tool_calls.as_ref().map(|calls| {
                    calls
                        .iter()
                        .map(|call| serde_json::json!({
                            "name": call.function.name,
                            "arguments": call.function.arguments,
                        }))
                        .collect::<Vec<_>>()
                }).unwrap_or_default(),
            }
        }));

        match self
            .resolve_response_to_action(response, world, memory, &[Box::new(crate::tool::IdentityTool)])
            .await?
        {
            DecodedTurn::Action(action) => {
                if let Some(final_answer) = self
                    .apply_action(action, world, memory, collected_deltas)
                    .await?
                {
                    Ok((final_answer, collected_deltas.clone()))
                } else {
                    self.host_terminal_fallback(
                        memory,
                        collected_deltas,
                        trace_reason,
                        Some("forced_turn_non_terminal_action".to_string()),
                    )
                    .await
                }
            }
            DecodedTurn::Anomaly(anomaly) => {
                let anomaly_reason = anomaly_trace_reason(&anomaly).to_string();
                self.record_decode_anomaly(memory, anomaly);
                self.host_terminal_fallback(
                    memory,
                    collected_deltas,
                    trace_reason,
                    Some(anomaly_reason),
                )
                .await
            }
        }
    }

    async fn host_terminal_fallback(
        &self,
        memory: &mut WorkingMemory,
        collected_deltas: &mut Vec<SyncAction>,
        trace_reason: &str,
        forced_turn_reason: Option<String>,
    ) -> Result<(String, Vec<SyncAction>), Error> {
        memory.technical_noise.push(serde_json::json!({
            "host_terminal_fallback": {
                "reason": trace_reason,
                "forced_turn_reason": forced_turn_reason,
                "answer": HOST_TERMINAL_FALLBACK_ANSWER,
            }
        }));

        match self
            .apply_action(
                AgentAction::Stop(HOST_TERMINAL_FALLBACK_ANSWER.to_string()),
                &crate::memory::InMemoryWorld { tickets: Vec::new() },
                memory,
                collected_deltas,
            )
            .await?
        {
            Some(final_answer) => Ok((final_answer, collected_deltas.clone())),
            None => Err(Error::Invariant(
                "host terminal fallback did not yield a terminal stop".to_string(),
            )),
        }
    }

    /// Recursively applies an action to the state.
    /// Captures stack mutations in the `deltas` list for future syncing.
    pub fn apply_action<'a>(
        &'a self,
        action: AgentAction,
        _world: &'a dyn HStackWorld,
        memory: &'a mut WorkingMemory,
        deltas: &'a mut Vec<SyncAction>,
    ) -> BoxFuture<'a, Result<Option<String>, Error>> {
        Box::pin(async move {
            match action {
                AgentAction::Stop(answer) => {
                    memory.push_message(Message {
                        role: crate::provider::Role::Assistant,
                        content: Some(answer.clone()),
                        tool_calls: None,
                        tool_call_id: None,
                        name: None,
                    });
                    Ok(Some(answer))
                }
                AgentAction::UpdateWorkingMemory(delta) => {
                    match delta {
                        WorkingMemoryDelta::AppendMessage(msg) => memory.push_message(msg),
                        WorkingMemoryDelta::AddTechnicalNoise(key, val) => {
                            memory.technical_noise.push(serde_json::json!({ key: val }));
                        }
                    }
                    Ok(None)
                }
                AgentAction::UpdateWorkspace(delta) => {
                    let event = memory.workspace.apply_delta(delta);
                    let _ = memory.workspace.materialize_allocation_plan();
                    memory.technical_noise.push(event);
                    Ok(None)
                }
                AgentAction::UpdateStack(sync_action) => {
                    // Safety gate: only collected if approved
                    self.control.validate_stack_action(&sync_action).await?;
                    memory.proposed_stack_actions.push(sync_action.clone());
                    deltas.push(sync_action);
                    Ok(None)
                }
                AgentAction::Compound(actions) => {
                    let mut last_stop = None;
                    for a in actions {
                        if let Some(stop) = self.apply_action(a, _world, memory, deltas).await? {
                            last_stop = Some(stop);
                        }
                    }
                    Ok(last_stop)
                }
            }
        })
    }
}

fn anomaly_trace_reason(anomaly: &DecodeAnomaly) -> &str {
    anomaly
        .payload
        .get("reason")
        .or_else(|| anomaly.payload.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("decode_anomaly")
}

fn build_tool_schemas(tools: &[Box<dyn Tool>]) -> Vec<hstack_core::provider::Tool> {
    tools.iter().map(|t| {
        hstack_core::provider::Tool {
            r#type: "function".to_string(),
            function: hstack_core::provider::ToolFunction {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters(),
            }
        }
    }).collect()
}

fn insert_tool_contract_message(messages: &mut Vec<Message>, tools: &[Box<dyn Tool>], forced_terminal: bool) {
    let refs = tools.iter().map(|tool| tool.as_ref()).collect::<Vec<_>>();
    insert_tool_contract_message_for_refs(messages, &refs, forced_terminal);
}

fn insert_tool_contract_message_for_refs(messages: &mut Vec<Message>, tools: &[&dyn Tool], forced_terminal: bool) {
    let tool_names = tools.iter().map(|tool| tool.name()).collect::<Vec<_>>();
    let mut content = String::new();
    content.push_str("TOOL CONTRACT\n");
    if forced_terminal {
        content.push_str("- Mandatory terminal mode is active.\n");
        content.push_str("- The only valid tool in this turn is `identity`.\n");
    } else {
        content.push_str("- Only the tools listed below are valid in this turn.\n");
    }
    if tool_names.is_empty() {
        content.push_str("- available_tools: []\n");
    } else {
        content.push_str("- available_tools: [");
        content.push_str(&tool_names.join(", "));
        content.push_str("]\n");
        for tool in tools {
            content.push_str("- tool ");
            content.push_str(tool.name());
            content.push_str(": ");
            content.push_str(tool.description());
            let parameter_summary = summarize_tool_parameters(tool.parameters());
            if !parameter_summary.is_empty() {
                content.push_str(" :: ");
                content.push_str(&parameter_summary);
            }
            content.push('\n');
        }
    }
    content.push_str("- Any other tool name is invalid and has no semantic effect.\n");

    // SPEC ANCHOR: keep a single leading system-role message. Adding a second
    // system message after historical tool or workspace messages breaks the
    // provider-visible ordering contract.
    if let Some(first_message) = messages.first_mut() {
        if matches!(first_message.role, Role::System) {
            match first_message.content.as_mut() {
                Some(existing) => {
                    existing.push_str("\n\n");
                    existing.push_str(&content);
                }
                None => first_message.content = Some(content),
            }
            return;
        }
    }

    messages.insert(0, Message {
        role: Role::System,
        content: Some(content),
        tool_calls: None,
        tool_call_id: None,
        name: None,
    });
}

fn summarize_tool_parameters(parameters: Value) -> String {
    let Some(properties) = parameters.get("properties").and_then(Value::as_object) else {
        return String::new();
    };

    let mut summaries = Vec::new();
    for (name, spec) in properties {
        if let Some(values) = spec.get("enum").and_then(Value::as_array) {
            let rendered = values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !rendered.is_empty() {
                summaries.push(format!("{} ∈ [{}]", name, rendered.join(", ")));
            }
        }
    }

    summaries.join("; ")
}

#[cfg(test)]
mod tests {
    use super::insert_tool_contract_message_for_refs;
    use crate::memory::{HStackWorld, WorkingMemory};
    use crate::provider::{Message, Role};
    use crate::tool::Tool;
    use crate::{AgentAction, Error};
    use async_trait::async_trait;
    use serde_json::Value;

    struct TestTool {
        name: &'static str,
        description: &'static str,
        parameters: Value,
    }

    #[async_trait]
    impl Tool for TestTool {
        fn name(&self) -> &str {
            self.name
        }

        fn description(&self) -> &str {
            self.description
        }

        fn parameters(&self) -> Value {
            self.parameters.clone()
        }

        async fn execute(&self, _args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
            Err(Error::Invariant("test tool should not execute".to_string()))
        }
    }

    #[test]
    fn tool_contract_lists_descriptions_and_enum_hints() {
        let manage = TestTool {
            name: "manage_app",
            description: "Controls app lifecycle.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "action": { "type": "string", "enum": ["open", "close"] },
                    "app_id": { "type": "string", "enum": ["file-tree", "editor"] }
                }
            }),
        };
        let microbash = TestTool {
            name: "microbash",
            description: "Runs constrained microbash commands against the configured sandboxed workspace.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                }
            }),
        };

        let mut messages = vec![Message {
            role: Role::System,
            content: Some("BASE".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }];
        let tools: Vec<&dyn Tool> = vec![&manage, &microbash];

        insert_tool_contract_message_for_refs(&mut messages, &tools, false);

        assert_eq!(messages.len(), 1);

        let contract = messages
            .first()
            .and_then(|message| message.content.as_deref())
            .unwrap_or("");
        assert!(contract.starts_with("BASE"));
        assert!(contract.contains("available_tools: [manage_app, microbash]"));
        assert!(contract.contains("tool manage_app: Controls app lifecycle."));
        assert!(contract.contains("action ∈ [open, close]"));
        assert!(contract.contains("app_id ∈ [file-tree, editor]"));
        assert!(contract.contains("tool microbash: Runs constrained microbash commands against the configured sandboxed workspace."));
    }
}
