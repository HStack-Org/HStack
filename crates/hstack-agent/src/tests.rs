#[cfg(test)]
mod tests {
    use crate::agent::Agent;
    use crate::memory::{InMemoryWorld, WorkingMemory};
    use crate::manager::SimpleContextManager;
    use crate::control::AllowAllControl;
    use crate::provider::{LlmProvider, Message, Role};
    use crate::tool::{IdentityTool, LightComputeTool, Tool};
    use crate::action::{AgentAction, WorkingMemoryDelta};
    use crate::error::Error;
    use hstack_core::sync::{SyncAction, SyncActionType};
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};

    struct MockProvider {
        pub responses: Arc<Mutex<Vec<Message>>>,
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
                Err(e) => return Err(Error::Internal(format!("mock provider mutex poisoned: {}", e))),
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
            Err(e) => panic!("agent run failed: {}", e),
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
            Err(e) => panic!("agent run failed: {}", e),
        };

        assert_eq!(answer, "Finished after search");
        assert!(memory.technical_noise.iter().any(|n| n.get("search_stack:test").is_some()));
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
            async fn execute(&self, _args: serde_json::Value, _world: &dyn crate::memory::HStackWorld) -> Result<AgentAction, Error> {
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
            Err(e) => panic!("agent run failed: {}", e),
        };

        assert_eq!(answer, "Done mutating");
        assert_eq!(deltas.len(), 1);
        assert_eq!(deltas[0].action_id, "act_1");
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
            Err(e) => panic!("agent run failed: {}", e),
        };

        assert_eq!(
            answer,
            "To the best of my internal knowledge, I cannot answer that from the available data."
        );
        assert!(memory.messages.is_empty());
        assert!(memory.technical_noise.iter().any(|n| {
            n.get("agent_runtime")
                .and_then(|payload| payload.get("reason"))
                .and_then(serde_json::Value::as_str)
                == Some("non_actionable_assistant_content")
        }));
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
            Err(e) => panic!("agent run failed: {}", e),
        };

        assert_eq!(answer, "Final answer from identity");
        assert!(memory.messages.is_empty());
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
            Err(e) => panic!("agent run failed: {}", e),
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
    async fn test_identity_tool_requires_non_empty_answer() {
        let tool = IdentityTool;
        let world = InMemoryWorld { tickets: Vec::new() };

        let action_res = tool.execute(serde_json::json!({}), &world).await;
        let err = match action_res {
            Ok(_) => panic!("expected identity to reject missing answer"),
            Err(e) => e,
        };

        match err {
            Error::Provider(msg) => assert!(msg.contains("identity requires a non-empty 'answer' string")),
            _ => panic!("unexpected error type from identity tool"),
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
            panic!("first acquire failed: {}", e);
        }
        assert!(start.elapsed().as_millis() < 50);

        // Second request should have ~1s wait
        let start = std::time::Instant::now();
        if let Err(e) = limiter.acquire(provider, 1, 0, &config).await {
            panic!("second acquire failed: {}", e);
        }
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 1000, "Expected at least 1s wait, got {}ms", elapsed);
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
            panic!("first token acquire failed: {}", e);
        }
        
        // Second 1000 tokens should wait ~1s
        let start = std::time::Instant::now();
        if let Err(e) = limiter.acquire(provider, 1, 1000, &config_fast).await {
            panic!("second token acquire failed: {}", e);
        }
        let elapsed = start.elapsed().as_millis();
        assert!(elapsed >= 1000, "Expected ~1s wait for tokens, got {}ms", elapsed);
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
                Err(e) => panic!("system clock error: {}", e),
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

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return hstack.add(input.a, input.b);",
                    "input": { "a": 2, "b": 3 }
                }),
                &world,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {}", e),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(key, payload)) => {
                assert_eq!(key, "light_compute");
                assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(true));
                assert_eq!(payload.get("result").and_then(serde_json::Value::as_f64), Some(5.0));
            }
            _ => panic!("unexpected action returned from light_compute"),
        }
    }

    #[tokio::test]
    async fn test_light_compute_timeout() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "while (true) {}"
                }),
                &world,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {}", e),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(_, payload)) => {
                assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(false));
                assert_eq!(
                    payload
                        .get("error")
                        .and_then(|e| e.get("type"))
                        .and_then(serde_json::Value::as_str),
                    Some("timeout")
                );
            }
            _ => panic!("unexpected action returned from light_compute"),
        }
    }

    #[tokio::test]
    async fn test_light_compute_forbidden_source() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return fetch('https://example.com');"
                }),
                &world,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {}", e),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(_, payload)) => {
                assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(false));
                assert_eq!(
                    payload
                        .get("error")
                        .and_then(|e| e.get("type"))
                        .and_then(serde_json::Value::as_str),
                    Some("forbidden")
                );
            }
            _ => panic!("unexpected action returned from light_compute"),
        }
    }

    #[tokio::test]
    async fn test_light_compute_stats_and_object_helpers() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return { mean: hstack.mean(input.values), med: hstack.median(input.values), picked: hstack.pick(input.obj, ['a', 'c']) };",
                    "input": { "values": [1, 2, 5, 8], "obj": { "a": 1, "b": 2, "c": 3 } }
                }),
                &world,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(_, payload)) => {
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
            _ => panic!("unexpected action returned from light_compute"),
        }
    }

    #[tokio::test]
    async fn test_light_compute_string_helpers() {
        let tool = LightComputeTool::new();
        let world = InMemoryWorld { tickets: Vec::new() };

        let action_res = tool
            .execute(
                serde_json::json!({
                    "code": "return hstack.replaceAll(hstack.trim(hstack.lower(input.txt)), 'world', 'hstack');",
                    "input": { "txt": "  HeLLo WORLD  " }
                }),
                &world,
            )
            .await;

        let action = match action_res {
            Ok(v) => v,
            Err(e) => panic!("light_compute failed: {e}"),
        };

        match action {
            AgentAction::UpdateWorkingMemory(WorkingMemoryDelta::AddTechnicalNoise(_, payload)) => {
                assert_eq!(payload.get("ok").and_then(serde_json::Value::as_bool), Some(true));
                assert_eq!(
                    payload.get("result").and_then(serde_json::Value::as_str),
                    Some("hello hstack")
                );
            }
            _ => panic!("unexpected action returned from light_compute"),
        }
    }
}
