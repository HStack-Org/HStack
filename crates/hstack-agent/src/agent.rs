use crate::memory::{HStackWorld, WorkingMemory};
use crate::manager::ContextManager;
use crate::control::AgentControlSystem;
use crate::tool::Tool;
use crate::action::{AgentAction, DecodeAnomaly, DecodedTurn, WorkingMemoryDelta};
use crate::provider::{LlmProvider, Message};
use crate::error::Error;
use hstack_core::sync::SyncAction;
use tracing::{debug, info, warn};
use futures::future::BoxFuture;

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
                return Err(Error::MaxIterations);
            }

            info!(iteration = iterations, "Starting agent reasoning step");

            // 1. Construct context (C_n)
            let messages = self.manager.construct_context(world, memory, &self.base_prompt).await?;

            // 2. Prepare tool schemas
            let tool_schemas: Vec<hstack_core::provider::Tool> = self.tools.iter().map(|t| {
                hstack_core::provider::Tool {
                    r#type: "function".to_string(),
                    function: hstack_core::provider::ToolFunction {
                        name: t.name().to_string(),
                        description: t.description().to_string(),
                        parameters: t.parameters(),
                    }
                }
            }).collect();

            // 3. Generate response from provider
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
            let decoded_turn = self.resolve_response_to_action(response.clone(), world).await?;

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
                    self.record_decode_anomaly(memory, anomaly);
                    on_progress(AgentProgressUpdate {
                        iteration: iterations,
                        phase: "decode_anomaly".to_string(),
                        working_memory: memory.clone(),
                    });
                    iterations += 1;
                }
            }
        }
    }

    /// Decodes raw provider output into the transition algebra used by the harness.
    ///
    /// This function should remain strict: only valid tool-driven actions or
    /// explicit runtime anomalies belong here. It must not treat free-form
    /// assistant prose as completion by itself.
    async fn resolve_response_to_action(&self, response: Message, world: &dyn HStackWorld) -> Result<DecodedTurn, Error> {
        let mut actions = Vec::new();
        let mut anomalies = Vec::new();
        let assistant_content = response
            .content
            .as_ref()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());

        let tool_calls = response.tool_calls.unwrap_or_default();

        if !tool_calls.is_empty() {
            if let Some(content) = assistant_content.as_ref() {
                anomalies.push(DecodeAnomaly::assistant_content_ignored(content.clone()));
            }
            for call in tool_calls {
                let tool = self.tools.iter().find(|t| t.name() == call.function.name);
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
                                continue;
                            }
                        };
                        actions.push(AgentAction::UpdateWorkingMemory(
                            WorkingMemoryDelta::AddTechnicalNoise(
                                format!("tool_call:{}", t.name()),
                                serde_json::json!({ "arguments": args.clone() }),
                            ),
                        ));
                        debug!(tool = %t.name(), "Executing tool");
                        match t.execute(args, world).await {
                            Ok(tool_action) => actions.push(tool_action),
                            Err(err) => {
                                warn!(tool = %t.name(), error = %err, "Tool execution failed; continuing with fallback trace");
                                anomalies.push(DecodeAnomaly::tool_execution_failed(
                                    t.name().to_string(),
                                    err.to_string(),
                                ));
                            }
                        }
                    }
                    None => {
                        warn!(tool = %call.function.name, "LLM requested unknown tool");
                        anomalies.push(DecodeAnomaly::unknown_tool(call.function.name));
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
                .map(|anomaly| Self::anomaly_to_action(anomaly))
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

    /// Recursively applies an action to the state.
    /// Captures stack mutations in the `deltas` list for future syncing.
    pub fn apply_action<'a>(
        &'a self,
        action: AgentAction,
        world: &'a dyn HStackWorld,
        memory: &'a mut WorkingMemory,
        deltas: &'a mut Vec<SyncAction>,
    ) -> BoxFuture<'a, Result<Option<String>, Error>> {
        Box::pin(async move {
            match action {
                AgentAction::Stop(answer) => Ok(Some(answer)),
                AgentAction::UpdateWorkingMemory(delta) => {
                    match delta {
                        WorkingMemoryDelta::AppendMessage(msg) => memory.messages.push(msg),
                        WorkingMemoryDelta::AddTechnicalNoise(key, val) => {
                            memory.technical_noise.push(serde_json::json!({ key: val }));
                        }
                    }
                    Ok(None)
                }
                AgentAction::UpdateStack(sync_action) => {
                    // Safety gate: only collected if approved
                    self.control.validate_stack_action(&sync_action).await?;
                    deltas.push(sync_action);
                    Ok(None)
                }
                AgentAction::Compound(actions) => {
                    let mut last_stop = None;
                    for a in actions {
                        if let Some(stop) = self.apply_action(a, world, memory, deltas).await? {
                            last_stop = Some(stop);
                        }
                    }
                    Ok(last_stop)
                }
            }
        })
    }
}
