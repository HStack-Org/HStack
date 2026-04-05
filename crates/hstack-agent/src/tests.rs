#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::agent::Agent;
    use crate::memory::{InMemoryWorld, WorkingMemory};
    use crate::manager::{ContextManager, SimpleContextManager};
    use crate::control::AllowAllControl;
    use crate::provider::{LlmProvider, Message, Role};
    use crate::tool::{compose_tools, CreateTicketTool, ExaSearchTool, FollowUpTool, IdentityTool, InspectAppTool, LightComputeTool, ManageAppTool, ScratchThought, ScratchpadEditTool, ScratchpadSearchTool, SearchStack, Tool};
    use crate::workspace::{compose_workspace_system_message, render_workspace_projection, short_term_messages, AppId, AppLifecycle, WorkspaceDelta};
    use crate::action::{AgentAction, WorkingMemoryDelta};
    use crate::error::Error;
    use crate::prompt::{build_base_prompt, AgentPromptProfile};
    use hstack_core::sync::{SyncAction, SyncActionType};
    use hstack_core::ticket::{Ticket, TicketPayload, TicketPriority, TicketType};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct MockProvider {
        pub responses: Arc<Mutex<Vec<Message>>>,
    }

    async fn apply_action_for_test(memory: &mut WorkingMemory, action: AgentAction) {
        let world = InMemoryWorld { tickets: Vec::new() };
        let agent = Agent {
            provider: Box::new(MockProvider { responses: Arc::new(Mutex::new(Vec::new())) }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![],
            base_prompt: "You are a helpful assistant.".to_string(),
        };
        let mut deltas = Vec::new();
        if let Err(e) = agent.apply_action(action, &world, memory, &mut deltas).await {
            panic!("apply_action failed: {e}");
        }
    }

    fn extract_light_compute_payload(action: AgentAction) -> serde_json::Value {
        match action {
            AgentAction::Compound(actions) => {
                let mut saw_workspace_update = false;
                for action in actions {
                    match action {
                        AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
                            if key == "light_compute" {
                                return payload;
                            }
                        }
                        AgentAction::UpdateWorkspace(_) => {
                            saw_workspace_update = true;
                        }
                        _ => {}
                    }
                }
                if !saw_workspace_update {
                    panic!("light_compute compound action did not include workspace update");
                }
                panic!("light_compute compound action did not include technical payload");
            }
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
                assert_eq!(key, "light_compute");
                payload
            }
            _ => panic!("unexpected action returned from light_compute"),
        }
    }

    #[async_trait]
    impl LlmProvider for MockProvider {
        async fn generate_content(
            &self,
            _messages: &[Message],
            _tools: Option<&[crate::provider::Tool]>,
        ) -> Result<Message, Error> {
            let mut resps = match self.responses.lock() {
                Ok(guard) => guard,
                Err(e) => return Err(Error::Invariant(format!("mock provider mutex poisoned: {e}"))),
            };
            if resps.is_empty() {
                return Err(Error::Provider("No more mock responses".to_string()));
            }
            Ok(resps.remove(0))
        }
    }

    #[tokio::test]
    async fn test_agent_run_completion() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        
        let mock_response = Message {
            role: Role::Assistant,
            content: Some("I have finished the task.".to_string()),
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_123".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "Task complete!"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider { 
                responses: Arc::new(Mutex::new(vec![mock_response])) 
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, deltas) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Task complete!");
        assert!(deltas.is_empty());
    }

    #[tokio::test]
    async fn test_agent_multi_turn_search() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let resp1 = Message {
            role: Role::Assistant,
            content: Some("Searching...".to_string()),
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_search".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "search_stack".to_string(),
                    arguments: r#"{"query": "test"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let resp2 = Message {
            role: Role::Assistant,
            content: Some("Found it.".to_string()),
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "Finished after search"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider { 
                responses: Arc::new(Mutex::new(vec![resp1, resp2])) 
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(crate::tool::SearchStack)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, _) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Finished after search");
        assert!(memory.technical_noise.iter().any(|n| n.get("search_stack:test").is_some()));
    }

    #[tokio::test]
    async fn test_agent_allows_repeating_the_same_tool_when_each_turn_is_structurally_valid() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let resp1 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_search_1".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "search_stack".to_string(),
                    arguments: r#"{"query": "weather in paris"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let resp2 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_search_2".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "search_stack".to_string(),
                    arguments: r#"{"query": "weather in paris"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let resp3 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "The local stack does not answer that question."}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![resp1, resp2, resp3]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(SearchStack)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, _) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "The local stack does not answer that question.");
        let search_call_count = memory
            .technical_noise
            .iter()
            .filter(|entry| entry.get("tool_call:search_stack").is_some())
            .count();
        assert_eq!(search_call_count, 2);
    }

    #[tokio::test]
    async fn test_multiple_tool_calls_in_single_response_are_structural_anomaly() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let resp1 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![
                crate::provider::ToolCall {
                    id: "call_create".to_string(),
                    r#type: "function".to_string(),
                    function: crate::provider::ToolFunctionCall {
                        name: "create_ticket".to_string(),
                        arguments: r#"{"type":"TASK","title":"Walk the dog"}"#.to_string(),
                    },
                },
                crate::provider::ToolCall {
                    id: "call_identity".to_string(),
                    r#type: "function".to_string(),
                    function: crate::provider::ToolFunctionCall {
                        name: "identity".to_string(),
                        arguments: r#"{"answer":"Done"}"#.to_string(),
                    },
                },
            ]),
            tool_call_id: None,
            name: None,
        };

        let resp2 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "forced_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer":"Blocked invalid multi-tool turn."}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![resp1, resp2]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(CreateTicketTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let (answer, deltas) = match agent.run(&world, &mut memory).await {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Blocked invalid multi-tool turn.");
        assert!(deltas.is_empty());
        assert!(memory.proposed_stack_actions.is_empty());
        assert_eq!(
            memory
                .technical_noise
                .iter()
                .filter(|entry| entry.get("tool_call:create_ticket").is_some())
                .count(),
            0
        );
        assert!(memory.technical_noise.iter().any(|entry| {
            entry.get("agent_runtime")
                .and_then(|payload| payload.get("reason"))
                .and_then(serde_json::Value::as_str)
                == Some("multiple_tool_calls_in_single_turn")
        }));
    }

    #[tokio::test]
    async fn test_agent_delta_collection() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        // Simulate a "Compound" action that updates stack and then stops
        // In a real scenario, this might come from a "CreateTicket" tool
        // But for testing the reasoning loop's collection logic, we'll manually wrap it in a mock response call
        
        let sync_action = SyncAction {
            action_id: "act_1".to_string(),
            r#type: SyncActionType::Create,
            entity_id: "t1".to_string(),
            entity_type: "TASK".to_string(),
            status: None,
            payload: None,
            notes: None,
            timestamp: "now".to_string(),
        };

        // We wrap the UpdateStack action in a Compound action manually in Agent logic if tools return it
        // Or simple tool execution
        
        struct MutationTool {
            pub action: SyncAction,
        }
        #[async_trait]
        impl crate::tool::Tool for MutationTool {
            fn name(&self) -> &str { "mutate" }
            fn description(&self) -> &str { "mutates stack" }
            fn parameters(&self) -> serde_json::Value { serde_json::json!({}) }
            async fn execute(&self, _args: serde_json::Value, _world: &dyn crate::memory::HStackWorld, _memory: &crate::memory::WorkingMemory) -> Result<AgentAction, Error> {
                Ok(AgentAction::UpdateStack(self.action.clone()))
            }
        }

        let resp1 = Message {
            role: Role::Assistant,
            content: Some("Mutating...".to_string()),
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_mut".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "mutate".to_string(),
                    arguments: "{}".to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let resp2 = Message {
            role: Role::Assistant,
            content: Some("Stopping...".to_string()),
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_stop".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "Done mutating"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider { 
                responses: Arc::new(Mutex::new(vec![resp1, resp2])) 
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![
                Box::new(IdentityTool), 
                Box::new(MutationTool { action: sync_action.clone() })
            ],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, deltas) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Done mutating");
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].action_id, "act_1");
    }

    #[tokio::test]
    async fn test_update_stack_is_recorded_in_agent_proposal_buffer() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        let agent = Agent {
            provider: Box::new(MockProvider { responses: Arc::new(Mutex::new(Vec::new())) }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![],
            base_prompt: "You are a helpful assistant.".to_string(),
        };
        let sync_action = SyncAction {
            action_id: "proposal-1".to_string(),
            r#type: SyncActionType::Create,
            entity_id: "ticket-1".to_string(),
            entity_type: "TASK".to_string(),
            status: None,
            payload: None,
            notes: None,
            timestamp: "now".to_string(),
        };
        let mut deltas = Vec::new();

        let result = agent
            .apply_action(AgentAction::UpdateStack(sync_action.clone()), &world, &mut memory, &mut deltas)
            .await;

        match result {
            Ok(None) => {}
            Ok(Some(stop)) => panic!("unexpected terminal result: {stop}"),
            Err(error) => panic!("apply_action failed: {error}"),
        }

        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].action_id, sync_action.action_id);
        assert_eq!(memory.proposed_stack_actions.len(), 1);
        assert_eq!(memory.proposed_stack_actions[0].action_id, sync_action.action_id);
    }

    #[tokio::test]
    async fn test_search_stack_sees_agent_proposals_before_host_mutation() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        memory.proposed_stack_actions.push(SyncAction {
            action_id: "proposal-1".to_string(),
            r#type: SyncActionType::Create,
            entity_id: "ticket-1".to_string(),
            entity_type: "TASK".to_string(),
            status: Some(hstack_core::ticket::TicketStatus::Idle),
            payload: Some(TicketPayload::Task {
                title: "Draft migration plan".to_string(),
                scheduled_time_iso: None,
                rrule: None,
                duration_minutes: None,
                status: None,
                priority: None,
                completed: Some(false),
            }),
            notes: Some("proposal only".to_string()),
            timestamp: "now".to_string(),
        });

        let action = SearchStack
            .execute(serde_json::json!({ "query": "migration" }), &world, &memory)
            .await;

        match action {
            Ok(AgentAction::UpdateWorkspace(WorkspaceDelta::PublishSearchResults { results, .. })) => {
                assert_eq!(results.len(), 1);
                assert_eq!(results[0].title, "Draft migration plan");
            }
            Ok(other) => panic!("unexpected action from search_stack: {other:?}"),
            Err(error) => panic!("search_stack failed: {error}"),
        }
    }

    #[tokio::test]
    async fn test_create_ticket_tool_emits_stack_proposal_actions() {
        let tool = CreateTicketTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action = tool
            .execute(
                serde_json::json!({
                    "type": "TASK",
                    "title": "Prepare release notes"
                }),
                &world,
                &memory,
            )
            .await;

        match action {
            Ok(AgentAction::UpdateStack(sync_action)) => {
                assert_eq!(sync_action.r#type, SyncActionType::Create);
                assert_eq!(sync_action.entity_type, "TASK");
            }
            Ok(AgentAction::Compound(actions)) => {
                assert!(!actions.is_empty());
                assert!(matches!(actions[0], AgentAction::UpdateStack(_)));
            }
            Ok(other) => panic!("unexpected action from create_ticket: {other:?}"),
            Err(error) => panic!("create_ticket failed: {error}"),
        }
    }

    #[tokio::test]
    async fn test_agent_no_actionable_output_does_not_fabricate_answer() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let resp1 = Message {
            role: Role::Assistant,
            content: Some("still nothing".to_string()),
            tool_calls: Some(vec![]),
            tool_call_id: None,
            name: None,
        };

        let resp2 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "To the best of my internal knowledge, I cannot answer that from the available data."}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![resp1, resp2]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, _) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(
            answer,
            "To the best of my internal knowledge, I cannot answer that from the available data."
        );
        assert!(memory.messages.iter().any(|message| {
            matches!(message.role, Role::Assistant)
                && message
                    .content
                    .as_deref()
                    == Some("To the best of my internal knowledge, I cannot answer that from the available data.")
        }));
        assert!(memory.technical_noise.iter().any(|n| {
            n.get("agent_runtime")
                .and_then(|payload| payload.get("reason"))
                .and_then(serde_json::Value::as_str)
                == Some("non_actionable_assistant_content")
        }));
        assert!(memory.technical_noise.iter().any(|n| n.get("identity_enforcement_turn").is_some()));
        assert!(!memory.technical_noise.iter().any(|n| n.get("agent_limit").is_some()));
    }

    #[tokio::test]
    async fn test_agent_ignores_assistant_narration_when_tool_call_exists() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let response = Message {
            role: Role::Assistant,
            content: Some("Hello! How can I help you today?".to_string()),
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "Final answer from identity"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![response]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, _) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Final answer from identity");
        assert!(memory.messages.iter().any(|message| {
            matches!(message.role, Role::Assistant)
                && message.content.as_deref() == Some("Final answer from identity")
        }));
        assert!(memory.technical_noise.iter().any(|n| {
            n.get("assistant_content_ignored")
                .and_then(|payload| payload.get("reason"))
                .and_then(serde_json::Value::as_str)
                == Some("content_with_tool_calls_has_no_semantic_effect")
        }));
    }

    #[tokio::test]
    async fn test_agent_rejects_malformed_tool_arguments_without_executing_tool() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let bad_response = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "bad_call".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: "{not valid json".to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let good_response = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "good_call".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "Recovered after malformed call"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![bad_response, good_response]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let run_res = agent.run(&world, &mut memory).await;
        let (answer, _) = match run_res {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Recovered after malformed call");
        assert!(memory.technical_noise.iter().any(|n| {
            n.get("tool_error:identity")
                .and_then(|payload| payload.get("type"))
                .and_then(serde_json::Value::as_str)
                == Some("invalid_arguments")
        }));
    }

    #[tokio::test]
    async fn test_identity_tool_requires_answer_field() {
        let tool = IdentityTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action_res = tool.execute(serde_json::json!({}), &world, &memory).await;
        let err = match action_res {
            Ok(_) => panic!("expected identity to reject missing answer"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("identity requires an 'answer' string")),
            _ => panic!("unexpected error type from identity tool"),
        }
    }

    #[tokio::test]
    async fn test_identity_tool_allows_explicit_empty_answer() {
        let tool = IdentityTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action = match tool.execute(serde_json::json!({
            "answer": ""
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("identity failed: {e}"),
        };

        match action {
            AgentAction::Stop(answer) => assert!(answer.is_empty()),
            _ => panic!("unexpected action returned from identity"),
        }
    }

    #[tokio::test]
    async fn test_follow_up_tool_requires_non_empty_question() {
        let tool = FollowUpTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({"question": "   "}), &world, &memory).await {
            Ok(_) => panic!("expected follow_up to reject empty question"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("follow_up requires a non-empty 'question' string")),
            _ => panic!("unexpected error type from follow_up tool"),
        }
    }

    #[tokio::test]
    async fn test_follow_up_tool_records_trace_without_stopping() {
        let tool = FollowUpTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action = match tool.execute(serde_json::json!({
            "question": "Which project do you want me to inspect?",
            "reason": "The request names multiple possible targets."
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("follow_up failed: {e}"),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
                assert_eq!(key, "follow_up");
                assert_eq!(payload.get("question").and_then(serde_json::Value::as_str), Some("Which project do you want me to inspect?"));
            }
            _ => panic!("unexpected action returned from follow_up"),
        }
    }

    #[tokio::test]
    async fn test_agent_can_record_follow_up_then_terminate_via_identity() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let resp1 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "follow_up_1".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "follow_up".to_string(),
                    arguments: r#"{"question":"Which project do you mean?","reason":"Multiple targets are possible."}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let resp2 = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "identity_1".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer":"Which project do you mean?"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![resp1, resp2]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(FollowUpTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let (answer, _) = match agent.run(&world, &mut memory).await {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "Which project do you mean?");
        assert!(memory.technical_noise.iter().any(|entry| entry.get("follow_up").is_some()));
    }

    #[test]
    fn test_compose_tools_unknown_name_is_configuration_error() {
        let err = match compose_tools(&["identity", "not_a_real_tool"]) {
            Ok(_) => panic!("expected compose_tools to reject unknown tool"),
            Err(err) => err,
        };

        match err {
            Error::Configuration(msg) => assert!(msg.contains("Unknown tool 'not_a_real_tool'")),
            other => panic!("unexpected error type from compose_tools: {other}"),
        }
    }

    #[tokio::test]
    async fn test_agent_max_iterations_requires_terminal_identity_action() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let mut repeated = (0..10)
            .map(|index| Message {
                role: Role::Assistant,
                content: None,
                tool_calls: Some(vec![crate::provider::ToolCall {
                    id: format!("call_follow_up_{index}"),
                    r#type: "function".to_string(),
                    function: crate::provider::ToolFunctionCall {
                        name: "follow_up".to_string(),
                        arguments: format!(
                            r#"{{"question":"Which target do you mean? #{index}","reason":"Need clarification before proceeding."}}"#
                        ),
                    },
                }]),
                tool_call_id: None,
                name: None,
            })
            .collect::<Vec<_>>();
        repeated.push(Message {
            role: Role::Assistant,
            content: Some(String::new()),
            tool_calls: Some(vec![]),
            tool_call_id: None,
            name: None,
        });

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(repeated))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(FollowUpTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let err = match agent.run(&world, &mut memory).await {
            Ok(result) => result,
            Err(err) => panic!("expected host fallback terminalization to succeed: {err}"),
        };

        let (answer, deltas) = err;
        assert_eq!(answer, "I could not complete a valid tool-grounded response for this turn.");
        assert!(deltas.is_empty());
        assert!(memory.technical_noise.iter().any(|entry| entry.get("agent_limit").is_some()));
        assert!(memory.technical_noise.iter().any(|entry| entry.get("forced_terminal_turn").is_some()));
        assert!(memory.technical_noise.iter().any(|entry| entry.get("host_terminal_fallback").is_some()));
    }

    #[tokio::test]
    async fn test_forced_terminal_turn_rejects_non_identity_tool_even_if_configured() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let responses = vec![Message {
            role: Role::Assistant,
            content: Some("free text without a tool call".to_string()),
            tool_calls: Some(vec![]),
            tool_call_id: None,
            name: None,
        }, Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "forced_follow_up".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "follow_up".to_string(),
                    arguments: r#"{"question":"Which project?"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        }];

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(responses))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(FollowUpTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let err = match agent.run(&world, &mut memory).await {
            Ok(result) => result,
            Err(err) => panic!("expected host fallback terminalization to succeed: {err}"),
        };

        let (answer, deltas) = err;
        assert_eq!(answer, "I could not complete a valid tool-grounded response for this turn.");
        assert!(deltas.is_empty());
        assert!(!memory.technical_noise.iter().any(|entry| entry.get("follow_up").is_some()));
        assert!(memory.technical_noise.iter().any(|entry| {
            entry.get("tool_error:follow_up")
                .and_then(|payload| payload.get("type"))
                .and_then(serde_json::Value::as_str)
                == Some("unknown_tool")
        }));
        assert!(memory.technical_noise.iter().any(|entry| entry.get("host_terminal_fallback").is_some()));
    }

    #[tokio::test]
    async fn test_agent_max_iterations_accepts_explicit_empty_identity_reply() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let mut repeated = (0..10)
            .map(|index| Message {
                role: Role::Assistant,
                content: None,
                tool_calls: Some(vec![crate::provider::ToolCall {
                    id: format!("call_follow_up_{index}"),
                    r#type: "function".to_string(),
                    function: crate::provider::ToolFunctionCall {
                        name: "follow_up".to_string(),
                        arguments: format!(
                            r#"{{"question":"Which target do you mean? #{index}","reason":"Need clarification before proceeding."}}"#
                        ),
                    },
                }]),
                tool_call_id: None,
                name: None,
            })
            .collect::<Vec<_>>();
        repeated.push(Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "forced_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": ""}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        });

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(repeated))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool), Box::new(FollowUpTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let (answer, deltas) = match agent.run(&world, &mut memory).await {
            Ok(result) => result,
            Err(err) => panic!("expected explicit empty identity reply to succeed: {err}"),
        };

        assert!(answer.is_empty());
        assert!(deltas.is_empty());
        assert!(memory.technical_noise.iter().any(|entry| entry.get("agent_limit").is_some()));
        assert!(memory.technical_noise.iter().any(|entry| entry.get("forced_terminal_turn").is_some()));
    }

    #[tokio::test]
    async fn test_agent_missing_identity_tool_is_invariant_violation() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let responses = (0..10)
            .map(|index| Message {
                role: Role::Assistant,
                content: Some(format!("loop {index}")),
                tool_calls: Some(vec![]),
                tool_call_id: None,
                name: None,
            })
            .collect::<Vec<_>>();

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(responses))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let err = match agent.run(&world, &mut memory).await {
            Ok(_) => panic!("expected agent run to fail without identity tool"),
            Err(err) => err,
        };

        match err {
            Error::Invariant(msg) => assert!(msg.contains("identity tool is missing")),
            other => panic!("unexpected error type from missing identity path: {other}"),
        }
    }

    #[tokio::test]
    async fn test_search_stack_requires_non_empty_query() {
        let tool = SearchStack;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({"query": "   "}), &world, &memory).await {
            Ok(_) => panic!("expected search_stack to reject empty query"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("search_stack requires a non-empty 'query' string")),
            _ => panic!("unexpected error type from search_stack"),
        }
    }

    #[tokio::test]
    async fn test_scratch_thought_requires_object_metadata() {
        let tool = ScratchThought;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({
            "thought": "reasoning",
            "metadata": ["not", "an", "object"]
        }), &world, &memory).await {
            Ok(_) => panic!("expected scratch_thought to reject non-object metadata"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("scratch_thought 'metadata' must be an object when provided")),
            _ => panic!("unexpected error type from scratch_thought"),
        }
    }

    #[tokio::test]
    async fn test_exa_search_rejects_invalid_num_results_before_network() {
        let tool = ExaSearchTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({
            "query": "rust",
            "num_results": 0
        }), &world, &memory).await {
            Ok(_) => panic!("expected exa_search to reject out-of-range num_results"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("exa_search 'num_results' must be between 1 and 25")),
            _ => panic!("unexpected error type from exa_search"),
        }
    }

    #[tokio::test]
    async fn test_light_compute_requires_object_input() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({
            "code": "return 1;",
            "input": [1, 2, 3]
        }), &world, &memory).await {
            Ok(_) => panic!("expected light_compute to reject non-object input"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("light_compute 'input' must be an object when provided")),
            _ => panic!("unexpected error type from light_compute"),
        }
    }

    #[tokio::test]
    async fn test_manage_app_focus_updates_workspace_state() {
        let tool = ManageAppTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let action = match tool.execute(serde_json::json!({
            "action": "focus",
            "app_id": "compute"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("manage_app failed: {e}"),
        };

        apply_action_for_test(&mut memory, action).await;

        assert_eq!(memory.workspace.dock.focused_app, crate::workspace::AppId::Compute);
    }

    #[tokio::test]
    async fn test_manage_app_open_close_pin_unpin_and_scroll() {
        let tool = ManageAppTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        memory.workspace.compute.history.extend((0..12).map(|idx| crate::workspace::ComputeRecord {
            summary: format!("run {idx}"),
            payload: serde_json::json!({ "idx": idx }),
        }));

        let open_action = match tool.execute(serde_json::json!({
            "action": "open",
            "app_id": "compute"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("manage_app open failed: {e}"),
        };
        apply_action_for_test(&mut memory, open_action).await;
        assert!(matches!(
            memory.workspace.compute.lifecycle,
            AppLifecycle::OpenUnmounted | AppLifecycle::OpenMounted | AppLifecycle::OpenMountedFocused
        ));

        let pin_action = match tool.execute(serde_json::json!({
            "action": "pin",
            "app_id": "compute"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("manage_app pin failed: {e}"),
        };
        apply_action_for_test(&mut memory, pin_action).await;
        assert!(memory.workspace.compute.pinned);

        let scroll_action = match tool.execute(serde_json::json!({
            "action": "scroll_down",
            "app_id": "compute",
            "lines": 3
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("manage_app scroll failed: {e}"),
        };
        apply_action_for_test(&mut memory, scroll_action).await;
        assert_eq!(memory.workspace.compute.viewport.start_line, 3);

        let unpin_action = match tool.execute(serde_json::json!({
            "action": "unpin",
            "app_id": "compute"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("manage_app unpin failed: {e}"),
        };
        apply_action_for_test(&mut memory, unpin_action).await;
        assert!(!memory.workspace.compute.pinned);

        let close_action = match tool.execute(serde_json::json!({
            "action": "close",
            "app_id": "compute"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("manage_app close failed: {e}"),
        };
        apply_action_for_test(&mut memory, close_action).await;
        assert_eq!(memory.workspace.compute.lifecycle, AppLifecycle::InstalledClosed);
    }

    #[tokio::test]
    async fn test_manage_app_rejects_invalid_app_id() {
        let tool = ManageAppTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({
            "action": "open",
            "app_id": "notes"
        }), &world, &memory).await {
            Ok(_) => panic!("expected manage_app to reject invalid app_id"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("manage_app requires a valid 'app_id'")),
            _ => panic!("unexpected error type from manage_app"),
        }
    }

    #[tokio::test]
    async fn test_scratchpad_edit_and_search_tools_round_trip() {
        let edit_tool = ScratchpadEditTool;
        let search_tool = ScratchpadSearchTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let edit_action = match edit_tool.execute(serde_json::json!({
            "operation": "append",
            "new_lines": ["important note for later"]
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("scratchpad_edit failed: {e}"),
        };

        apply_action_for_test(&mut memory, edit_action).await;

        let search_action = match search_tool.execute(serde_json::json!({
            "query": "important"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("scratchpad_search failed: {e}"),
        };

        match search_action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
                assert_eq!(key, "scratchpad_search:important");
                let matches = payload.get("matches").and_then(serde_json::Value::as_array);
                assert!(matches.map(|items| !items.is_empty()).unwrap_or(false));
            }
            _ => panic!("unexpected action returned from scratchpad_search"),
        }
    }

    #[tokio::test]
    async fn test_scratchpad_edit_replace_insert_and_delete_work_like_diff_operations() {
        let edit_tool = ScratchpadEditTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        memory.workspace.scratchpad.document_lines = vec![
            "# Scratchpad".to_string(),
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
        ];

        let replace_action = match edit_tool.execute(serde_json::json!({
            "operation": "replace",
            "start_line": 1,
            "delete_count": 2,
            "new_lines": ["beta-1", "beta-2"]
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("scratchpad_edit replace failed: {e}"),
        };
        apply_action_for_test(&mut memory, replace_action).await;
        assert_eq!(
            memory.workspace.scratchpad.document_lines,
            vec![
                "# Scratchpad".to_string(),
                "beta-1".to_string(),
                "beta-2".to_string(),
                "gamma".to_string(),
            ]
        );

        let insert_action = match edit_tool.execute(serde_json::json!({
            "operation": "insert",
            "start_line": 3,
            "new_lines": ["inserted"]
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("scratchpad_edit insert failed: {e}"),
        };
        apply_action_for_test(&mut memory, insert_action).await;
        assert_eq!(memory.workspace.scratchpad.document_lines[3], "inserted");

        let delete_action = match edit_tool.execute(serde_json::json!({
            "operation": "delete",
            "start_line": 2,
            "delete_count": 2
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("scratchpad_edit delete failed: {e}"),
        };
        apply_action_for_test(&mut memory, delete_action).await;
        assert_eq!(
            memory.workspace.scratchpad.document_lines,
            vec![
                "# Scratchpad".to_string(),
                "beta-1".to_string(),
                "gamma".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn test_scratchpad_edit_rejects_non_string_new_lines() {
        let edit_tool = ScratchpadEditTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match edit_tool.execute(serde_json::json!({
            "operation": "append",
            "new_lines": ["ok", 42]
        }), &world, &memory).await {
            Ok(_) => panic!("expected scratchpad_edit to reject non-string new_lines"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("scratchpad_edit 'new_lines' must contain strings")),
            _ => panic!("unexpected error type from scratchpad_edit"),
        }
    }

    #[tokio::test]
    async fn test_inspect_app_reports_visible_scratchpad_viewport() {
        let inspect_tool = InspectAppTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        memory.workspace.apply_delta(WorkspaceDelta::ScratchpadAppend {
            thought: "viewport check".to_string(),
            metadata: serde_json::Value::Null,
        });

        let action = match inspect_tool.execute(serde_json::json!({
            "app_id": "scratchpad"
        }), &world, &memory).await {
            Ok(action) => action,
            Err(e) => panic!("inspect_app failed: {e}"),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
                assert_eq!(key, "inspect_app:scratchpad");
                let visible = payload.get("visible").and_then(serde_json::Value::as_array);
                assert!(visible.map(|items| !items.is_empty()).unwrap_or(false));
            }
            _ => panic!("unexpected action returned from inspect_app"),
        }
    }

    #[tokio::test]
    async fn test_inspect_app_rejects_invalid_app_id() {
        let inspect_tool = InspectAppTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match inspect_tool.execute(serde_json::json!({
            "app_id": "notes"
        }), &world, &memory).await {
            Ok(_) => panic!("expected inspect_app to reject invalid app_id"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("inspect_app requires a valid 'app_id'")),
            _ => panic!("unexpected error type from inspect_app"),
        }
    }

    #[tokio::test]
    async fn test_scratchpad_search_rejects_empty_query() {
        let search_tool = ScratchpadSearchTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match search_tool.execute(serde_json::json!({
            "query": "   "
        }), &world, &memory).await {
            Ok(_) => panic!("expected scratchpad_search to reject empty query"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("scratchpad_search requires a non-empty 'query' string")),
            _ => panic!("unexpected error type from scratchpad_search"),
        }
    }

    #[tokio::test]
    async fn test_short_term_memory_is_bounded_by_budget() {
        let mut memory = WorkingMemory::new();
        memory.workspace.budget.short_term_budget = 40;

        for idx in 0..10 {
            memory.push_message(Message {
                role: Role::User,
                content: Some(format!("message number {idx}")),
                tool_calls: None,
                tool_call_id: None,
                name: None,
            });
        }

        assert!(memory.messages.len() < 10);
        assert!(!memory.messages.is_empty());
    }

    #[tokio::test]
    async fn test_short_term_kernel_preserves_latest_user_goal() {
        let mut memory = WorkingMemory::new();
        memory.workspace.budget.short_term_budget = 20;

        memory.push_message(Message {
            role: Role::User,
            content: Some("finish the websocket migration without losing the initial goal".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
        memory.push_message(Message {
            role: Role::Assistant,
            content: Some("intermediate reasoning that would otherwise crowd the window".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        let rendered = short_term_messages(&memory);
        assert!(rendered.iter().any(|message| {
            matches!(message.role, Role::User)
                && message
                    .content
                    .as_deref()
                    .map(|content| content.contains("initial goal"))
                    .unwrap_or(false)
        }));
    }

    #[tokio::test]
    async fn test_near_event_kernel_surfaces_scheduled_and_urgent_items() {
        let mut memory = WorkingMemory::new();
        let scheduled_ticket = Ticket::new(
            "Soon event".to_string(),
            TicketType::Event,
            TicketPayload::Event {
                title: "Soon event".to_string(),
                scheduled_time_iso: Some((chrono::Utc::now() + chrono::Duration::hours(4)).to_rfc3339()),
                rrule: None,
                duration_minutes: Some(30),
                location: None,
                status: None,
                priority: Some(TicketPriority::Medium),
                completed: Some(false),
            },
            None,
        );
        let urgent_ticket = Ticket::new(
            "Urgent task".to_string(),
            TicketType::Task,
            TicketPayload::Task {
                title: "Urgent task".to_string(),
                scheduled_time_iso: None,
                rrule: None,
                duration_minutes: None,
                status: None,
                priority: Some(TicketPriority::Urgent),
                completed: Some(false),
            },
            None,
        );

        memory.workspace.refresh_near_events(&[scheduled_ticket, urgent_ticket]);

        assert_eq!(memory.workspace.near_events.len(), 2);
        assert!(memory.workspace.near_events.iter().any(|item| item.reason == "scheduled_within_72h"));
        assert!(memory.workspace.near_events.iter().any(|item| item.reason == "urgent_priority"));
    }

    #[tokio::test]
    async fn test_near_event_kernel_excludes_far_non_urgent_items() {
        let mut memory = WorkingMemory::new();
        let far_ticket = Ticket::new(
            "Far event".to_string(),
            TicketType::Event,
            TicketPayload::Event {
                title: "Far event".to_string(),
                scheduled_time_iso: Some((chrono::Utc::now() + chrono::Duration::hours(120)).to_rfc3339()),
                rrule: None,
                duration_minutes: Some(45),
                location: None,
                status: None,
                priority: Some(TicketPriority::Low),
                completed: Some(false),
            },
            None,
        );

        memory.workspace.refresh_near_events(&[far_ticket]);

        assert!(memory.workspace.near_events.is_empty());
    }

    #[tokio::test]
    async fn test_materialized_allocation_persists_mounted_apps() {
        let mut memory = WorkingMemory::new();
        let _ = memory.workspace.apply_delta(WorkspaceDelta::OpenApp(AppId::StackSearch));
        let _ = memory.workspace.apply_delta(WorkspaceDelta::FocusApp(AppId::StackSearch));

        let plan = memory.workspace.materialize_allocation_plan();

        assert!(plan.mounted_apps.contains(&AppId::StackSearch));
        assert!(memory.workspace.dock.mounted_apps.contains(&AppId::StackSearch));
        assert_eq!(memory.workspace.stack_search.lifecycle, AppLifecycle::OpenMountedFocused);
    }

    #[tokio::test]
    async fn test_workspace_context_includes_dock_and_focused_app_viewport() {
        let mut memory = WorkingMemory::new();
        let _ = memory.workspace.apply_delta(WorkspaceDelta::ScratchpadAppend {
            thought: "workspace focus note".to_string(),
            metadata: serde_json::Value::Null,
        });
        let _ = memory.workspace.apply_delta(WorkspaceDelta::FocusApp(AppId::Scratchpad));

        let tickets = vec![Ticket {
            id: "ticket-1".to_string(),
            title: "Visible projected task".to_string(),
            r#type: TicketType::Task,
            status: hstack_core::ticket::TicketStatus::Idle,
            payload: TicketPayload::Task {
                title: "Visible projected task".to_string(),
                scheduled_time_iso: None,
                rrule: None,
                duration_minutes: None,
                status: None,
                priority: None,
                completed: Some(false),
            },
            notes: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }];

        let system = compose_workspace_system_message(
            "BASE PROMPT",
            &memory,
            &tickets,
            &hstack_core::settings::UserSettings::default(),
            &[],
        );
        assert!(system.contains("DOCK"));
        assert!(system.contains("SCRATCHPAD"));
        assert!(system.contains("workspace focus note"));
        assert!(system.contains("PROJECTED STACK"));
        assert!(system.contains("Visible projected task"));
    }

    #[tokio::test]
    async fn test_context_manager_does_not_duplicate_short_term_history_in_system_prompt() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();
        memory.push_message(Message {
            role: Role::User,
            content: Some("single question".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });

        let manager = SimpleContextManager;
        let messages = match manager.construct_context(&world, &memory, "BASE PROMPT").await {
            Ok(messages) => messages,
            Err(e) => panic!("construct_context failed: {e}"),
        };

        let system_content = messages[0].content.as_deref().unwrap_or_default();
        assert!(!system_content.contains("single question"));

        let short_term_occurrences = messages
            .iter()
            .filter_map(|message| message.content.as_deref())
            .filter(|content| *content == "single question")
            .count();
        assert_eq!(short_term_occurrences, 1);
    }

    #[tokio::test]
    async fn test_terminal_answer_is_persisted_to_working_memory() {
        let world = InMemoryWorld { tickets: Vec::new() };
        let mut memory = WorkingMemory::new();

        let response = Message {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(vec![crate::provider::ToolCall {
                id: "call_id".to_string(),
                r#type: "function".to_string(),
                function: crate::provider::ToolFunctionCall {
                    name: "identity".to_string(),
                    arguments: r#"{"answer": "terminal answer"}"#.to_string(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };

        let agent = Agent {
            provider: Box::new(MockProvider {
                responses: Arc::new(Mutex::new(vec![response]))
            }),
            manager: Box::new(SimpleContextManager),
            control: Box::new(AllowAllControl),
            tools: vec![Box::new(IdentityTool)],
            base_prompt: "You are a helpful assistant.".to_string(),
        };

        let (answer, _) = match agent.run(&world, &mut memory).await {
            Ok(v) => v,
            Err(e) => panic!("agent run failed: {e}"),
        };

        assert_eq!(answer, "terminal answer");
        assert!(memory.messages.iter().any(|message| {
            matches!(message.role, Role::Assistant)
                && message.content.as_deref() == Some("terminal answer")
        }));
    }

    #[tokio::test]
    async fn test_workspace_projection_matches_visible_workspace_regions() {
        let mut memory = WorkingMemory::new();
        memory.push_message(Message {
            role: Role::User,
            content: Some("show me what you can see".to_string()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        });
        let _ = memory.workspace.apply_delta(WorkspaceDelta::ScratchpadAppend {
            thought: "projection note".to_string(),
            metadata: serde_json::Value::Null,
        });

        let projection = render_workspace_projection(&memory, &[]);
        assert!(projection.contains("SHORT-TERM KERNEL"));
        assert!(projection.contains("user: show me what you can see"));
        assert!(projection.contains("DOCK"));
        assert!(projection.contains("SCRATCHPAD"));
        assert!(projection.contains("projection note"));
    }

    #[tokio::test]
    async fn test_dock_context_shows_state_not_action_catalog() {
        let mut memory = WorkingMemory::new();
        let _ = memory.workspace.apply_delta(WorkspaceDelta::PinApp(AppId::Compute));
        let _ = memory.workspace.apply_delta(WorkspaceDelta::OpenApp(AppId::Compute));
        let _ = memory.workspace.materialize_allocation_plan();

        let system = compose_workspace_system_message(
            "BASE PROMPT",
            &memory,
            &[],
            &hstack_core::settings::UserSettings::default(),
            &[],
        );
        assert!(system.contains("pinned: [scratchpad, compute]"));
        assert!(system.contains("mounted:"));
        assert!(!system.contains("actions: open close focus pin unpin scroll inspect search edit"));
    }

    #[tokio::test]
    async fn test_prompt_explicitly_separates_local_search_web_search_and_compute() {
        let prompt = build_base_prompt(AgentPromptProfile::DebugInteractive);
        assert!(prompt.contains("If the answer should come from the user's own HStack items, use `search_stack`."));
        assert!(prompt.contains("If the answer should come from the public web, documentation, or current external facts, use `exa_search` when available."));
        assert!(prompt.contains("If the answer should come from deterministic transformation of already-available information, use `light_compute`."));
        assert!(prompt.contains("Do not use a local-stack tool to answer a world-knowledge question."));
        assert!(prompt.contains("Use `follow_up` only to record the needed clarification before the final `identity` reply."));
    }

    #[tokio::test]
    async fn test_workspace_projection_hides_websearch_when_exa_unavailable() {
        let mut memory = WorkingMemory::new();
        let _ = memory.workspace.apply_delta(WorkspaceDelta::OpenApp(AppId::WebSearch));
        let projection = render_workspace_projection(&memory, &[]);

        assert!(!projection.contains("WEBSEARCH APP"));
        assert!(!projection.contains("websearch ::"));
    }

    #[tokio::test]
    async fn test_manage_app_rejects_unavailable_websearch() {
        let tool = ManageAppTool;
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let err = match tool.execute(serde_json::json!({
            "action": "open",
            "app_id": "websearch"
        }), &world, &memory).await {
            Ok(_) => panic!("expected manage_app to reject unavailable websearch"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("manage_app cannot operate on unavailable app 'websearch'")),
            _ => panic!("unexpected error type from manage_app"),
        }
    }

    #[tokio::test]
    async fn test_local_rate_limiter_rps_shaping() {
        use crate::rate_limiter::{LocalRateLimiter, RateLimitConfig, RateLimiter};
        let config = RateLimitConfig {
            requests_per_second: 1,
            requests_per_minute: 60,
            tokens_per_minute: 1000,
        };
        let limiter = LocalRateLimiter::new();
        let provider = "test_provider";

        // First request should be instant
        let start = std::time::Instant::now();
        if let Err(e) = limiter.acquire(provider, 1, 0, &config).await {
            panic!("first acquire failed: {e}");
        }
        assert!(start.elapsed().as_millis() < 50);

        // Second request should have ~1s wait
        let start = std::time::Instant::now();
        if let Err(e) = limiter.acquire(provider, 1, 0, &config).await {
            panic!("second acquire failed: {e}");
        }
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 1000, "Expected at least 1s wait, got {elapsed}ms");
        assert!(elapsed < 1200); // 1s + jitter
    }

    #[tokio::test]
    async fn test_local_rate_limiter_tpm_shaping() {
        use crate::rate_limiter::{LocalRateLimiter, RateLimitConfig, RateLimiter};
        let limiter = LocalRateLimiter::new();
        let provider = "test_provider_fast";

        let config_fast = RateLimitConfig {
            requests_per_second: 100,
            requests_per_minute: 1000,
            tokens_per_minute: 60000, // 1000 tokens per second
        };
        
        // Use 1000 tokens. Should be instant.
        if let Err(e) = limiter.acquire(provider, 1, 1000, &config_fast).await {
            panic!("first token acquire failed: {e}");
        }
        
        // Second 1000 tokens should wait ~1s
        let start = std::time::Instant::now();
        if let Err(e) = limiter.acquire(provider, 1, 1000, &config_fast).await {
            panic!("second token acquire failed: {e}");
        }
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 1000, "Expected ~1s wait for tokens, got {elapsed}ms");
    }

    #[tokio::test]
    async fn test_local_rate_limiter_max_delay() {
        use crate::rate_limiter::{LocalRateLimiter, RateLimitConfig, RateLimiter};
        use crate::error::Error;
        let config = RateLimitConfig {
            requests_per_second: 1, // 1 req/s
            requests_per_minute: 60,
            tokens_per_minute: 1000,
        };
        let limiter = LocalRateLimiter::new();
        
        // Max delay is 30 minutes (1800 seconds).
        // If we book more than 1800s, the request should be rejected.
        
        {
            let mut state = limiter.state.lock().await;
            let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
                Ok(d) => d.as_secs_f64(),
                Err(e) => panic!("system clock error: {e}"),
            };
            state.insert("rl:prov:greedy:batch:rps".to_string(), now + 2000.0);
        }

        let result = limiter.acquire("greedy", 1, 0, &config).await;
        assert!(result.is_err());
        let err = match result {
            Ok(_) => panic!("expected rate limit error"),
            Err(e) => e,
        };
        match err {
            Error::RateLimitExceeded { wait_time } => assert!(wait_time > 1900.0),
            _ => panic!("Expected RateLimitExceeded error"),
        }
    }

    #[tokio::test]
    async fn test_light_compute_success_and_native_op() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return hstack.add(input.a, input.b);",
                    "input": { "a": 2, "b": 3 }
                }),
                &world,
                &memory,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        let payload = extract_light_compute_payload(action);
        assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(true));
        assert_eq!(payload.get("result").and_then(serde_json::Value::as_f64), Some(5.0));
    }

    #[tokio::test]
    async fn test_light_compute_timeout() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "while (true) {}"
                }),
                &world,
                &memory,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        let payload = extract_light_compute_payload(action);
        assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(false));
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("type"))
                .and_then(serde_json::Value::as_str),
            Some("timeout")
        );
    }

    #[tokio::test]
    async fn test_light_compute_forbidden_source() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return fetch('https://example.com');"
                }),
                &world,
                &memory,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        let payload = extract_light_compute_payload(action);
        assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(false));
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("type"))
                .and_then(serde_json::Value::as_str),
            Some("forbidden")
        );
    }

    #[tokio::test]
    async fn test_light_compute_stats_and_object_helpers() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return { mean: hstack.mean(input.values), med: hstack.median(input.values), picked: hstack.pick(input.obj, ['a', 'c']) };",
                    "input": { "values": [1, 2, 5, 8], "obj": { "a": 1, "b": 2, "c": 3 } }
                }),
                &world,
                &memory,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        let payload = extract_light_compute_payload(action);
        assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(true));
        assert_eq!(
            payload
                .get("result")
                .and_then(|r| r.get("mean"))
                .and_then(serde_json::Value::as_f64),
            Some(4.0)
        );
        assert_eq!(
            payload
                .get("result")
                .and_then(|r| r.get("med"))
                .and_then(serde_json::Value::as_f64),
            Some(3.5)
        );
        assert_eq!(
            payload
                .get("result")
                .and_then(|r| r.get("picked"))
                .and_then(|o| o.get("a"))
                .and_then(serde_json::Value::as_i64),
            Some(1)
        );
        assert_eq!(
            payload
                .get("result")
                .and_then(|r| r.get("picked"))
                .and_then(|o| o.get("c"))
                .and_then(serde_json::Value::as_i64),
            Some(3)
        );
    }

    #[tokio::test]
    async fn test_light_compute_string_helpers() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };
        let memory = WorkingMemory::new();

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return hstack.replaceAll(hstack.trim(hstack.lower(input.txt)), 'world', 'hstack');",
                    "input": { "txt": "  HeLLo WORLD  " }
                }),
                &world,
                &memory,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        let payload = extract_light_compute_payload(action);
        assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(true));
        assert_eq!(
            payload.get("result").and_then(serde_json::Value::as_str),
            Some("hello hstack")
        );
    }
}
