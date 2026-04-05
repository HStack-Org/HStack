use crate::error::Error;
use crate::provider::{
    gemini::generate_gemini_content, openai_compat::generate_openai_content, Message,
    ProviderConfig, ProviderKind, Role, Tool,
};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use tracing::{debug, warn};

pub type ToolExecutor = Box<
    dyn Fn(String, Value) -> Pin<Box<dyn Future<Output = Result<String, Error>> + Send>>
        + Send
        + Sync,
>;

pub type ContextRefreshFn = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<String, Error>> + Send>>
        + Send
        + Sync,
>;

pub async fn chat_loop(
    config: &ProviderConfig,
    messages: &mut Vec<Message>,
    tools: &[Tool],
    tool_executor: &ToolExecutor,
    context_refresh: Option<&ContextRefreshFn>,
) -> Result<Message, Error> {
    let max_iterations = 5;
    let mut iterations = 0;

    loop {
        if iterations >= max_iterations {
            return Err(Error::MaxIterations);
        }

        let response_result = match config.kind {
            ProviderKind::OpenAiCompatible => {
                generate_openai_content(config, messages, Some(tools)).await
            }
            ProviderKind::Gemini => generate_gemini_content(config, messages, Some(tools)).await,
        };

        let response = match response_result {
            Ok(msg) => msg,
            Err(e) => return Err(e),
        };

        // Only native structured tool calls are actionable.
        let tool_calls = response.tool_calls.clone().unwrap_or_default();

        if tool_calls.is_empty() {
            messages.push(response.clone());
            return Ok(response);
        }

        // We have tool calls
        let mut resp_with_tools = response.clone();
        resp_with_tools.tool_calls = Some(tool_calls.clone());
        messages.push(resp_with_tools);

        for call in tool_calls {
            let args = serde_json::from_str::<Value>(&call.function.arguments).map_err(|e| {
                Error::ProviderContract(format!(
                    "tool call '{}' carried malformed JSON arguments: {e}",
                    call.function.name
                ))
            })?;

            let tool_result = tool_executor(call.function.name.clone(), args).await;

            let content = match tool_result {
                Ok(s) => s,
                Err(e) => format!("Error executing tool: {e:?}"),
            };

            messages.push(Message {
                role: Role::Tool,
                content: Some(content),
                tool_calls: None,
                tool_call_id: Some(call.id.clone()),
                name: Some(call.function.name.clone()),
            });
        }

        // After executing tool calls, refresh system context if refresh function provided
        if let Some(refresh_fn) = context_refresh {
            // Check if there's a system message to replace
            if let Some(system_msg_idx) = messages.iter().position(|m| m.role == Role::System) {
                // Generate fresh context and replace
                match refresh_fn().await {
                    Ok(fresh_prompt) => {
                        messages[system_msg_idx] = Message {
                            role: Role::System,
                            content: Some(fresh_prompt),
                            tool_calls: None,
                            tool_call_id: None,
                            name: None,
                        };
                        debug!("system context refreshed after tool execution");
                    }
                    Err(e) => {
                        warn!(error = %e, "failed to refresh system context after tool execution");
                    }
                }
            }
        }

        iterations += 1;
    }
}