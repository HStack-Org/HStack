use super::{Message, ProviderConfig, Role, Tool, ToolCall as ProviderToolCall};
use crate::error::Error;
use serde::{Deserialize, Serialize};
use serde_json::from_str;
use tracing::{debug, enabled, trace, Level};

#[derive(Serialize)]
struct OpenAiToolFunctionCall<'a> {
    name: &'a str,
    arguments: &'a str,
}

#[derive(Serialize)]
struct OpenAiToolCall<'a> {
    id: &'a str,
    #[serde(rename = "type")]
    tool_type: &'a str,
    function: OpenAiToolFunctionCall<'a>,
}


#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall<'a>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
}

#[derive(Serialize)]
struct OpenAiChatRequest<'a> {
    model: &'a str,
    messages: Vec<OpenAiMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [Tool]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
}

#[derive(Deserialize)]
struct OpenAiChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: Message,
}

fn map_openai_tool_calls<'a>(tool_calls: Option<&'a [ProviderToolCall]>) -> Option<Vec<OpenAiToolCall<'a>>> {
    tool_calls.map(|calls| {
        calls
            .iter()
            .map(|call| OpenAiToolCall {
                id: call.id.as_str(),
                tool_type: call.r#type.as_str(),
                function: OpenAiToolFunctionCall {
                    name: call.function.name.as_str(),
                    arguments: call.function.arguments.as_str(),
                },
            })
            .collect()
    })
}

fn truncate_for_log(body: &str) -> &str {
    const LIMIT: usize = 2000;
    if body.len() <= LIMIT {
        body
    } else {
        &body[..LIMIT]
    }
}

pub async fn generate_openai_content(
    config: &ProviderConfig,
    messages: &[Message],
    tools: Option<&[Tool]>,
) -> Result<Message, Error> {
    let client = reqwest::Client::new();
    let api_url = format!("{}/chat/completions", config.endpoint.trim_end_matches('/'));


    let mut mapped_messages = Vec::new();
    for m in messages {
        mapped_messages.push(OpenAiMessage {
            role: match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            },
            content: m.content.as_deref(),
            tool_calls: if matches!(m.role, Role::Assistant) {
                map_openai_tool_calls(m.tool_calls.as_deref())
            } else {
                None
            },
            tool_call_id: if m.role == Role::Tool {
                m.tool_call_id.as_deref()
            } else {
                None
            },
            name: if m.role == Role::Tool {
                m.name.as_deref()
            } else {
                None
            },
        });
    }

    let message_count = mapped_messages.len();

    let request = OpenAiChatRequest {
        model: &config.model_name,
        messages: mapped_messages,
        temperature: Some(0.7),
        tools,
        tool_choice: if tools.is_some() { Some("auto") } else { None },
    };

    let mut headers = reqwest::header::HeaderMap::new();

    debug!(api_url = %api_url, model = %config.model_name, message_count, has_tools = tools.is_some(), "sending OpenAI-compatible chat request");
    if enabled!(Level::TRACE) {
        trace!(request = %serde_json::to_string(&request).unwrap_or_else(|_| "<serialization failed>".to_string()), "OpenAI-compatible request payload");
    }

    headers.insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("http-referer"),
        reqwest::header::HeaderValue::from_static("https://hstack.app"),
    );
    headers.insert(
        reqwest::header::HeaderName::from_static("x-title"),
        reqwest::header::HeaderValue::from_static("HStack"),
    );

    if !config.api_key.is_empty() {
        let auth_str = if config.api_key.starts_with("Bearer ") {
            config.api_key.clone()
        } else {
            format!("Bearer {}", config.api_key)
        };
        match reqwest::header::HeaderValue::from_str(&auth_str) {
            Ok(val) => {
                headers.insert(reqwest::header::AUTHORIZATION, val);
            }
            Err(e) => {
                return Err(Error::Header(format!("Invalid API key format: {e}")));
            }
        }
    }

    let response_result = client
        .post(&api_url)
        .headers(headers)
        .json(&request)
        .send()
        .await;

    let response = match response_result {
        Ok(res) => res,
        Err(e) => return Err(Error::Network(e.to_string())),
    };

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = match response.text().await {
            Ok(t) => t,
            Err(_) => "Could not read error body".to_string(),
        };
        debug!(status, body = %body, "OpenAI-compatible API returned an error response");
        return Err(Error::Api { status, body });
    }

    let response_body = match response.text().await {
        Ok(body) => body,
        Err(e) => {
            debug!(error = %e, "failed to read OpenAI-compatible response body");
            return Err(Error::ProviderContract(format!("Malformed provider response: could not read response body: {e}")));
        }
    };

    let response_data: OpenAiChatResponse = match from_str(&response_body) {
        Ok(data) => data,
        Err(e) => {
            debug!(error = %e, body = %truncate_for_log(&response_body), "failed to parse OpenAI-compatible response body");
            return Err(Error::ProviderContract(format!(
                "Malformed provider response: {e}. Body: {}",
                truncate_for_log(&response_body)
            )));
        }
    };

    let mut choices = response_data.choices;
    if choices.is_empty() {
        return Err(Error::ProviderContract("API returned empty choices".to_string()));
    }

    let msg = choices.remove(0).message;
    if enabled!(Level::TRACE) {
        trace!(message = ?msg, "received OpenAI-compatible response message");
    }
    Ok(msg)
}
