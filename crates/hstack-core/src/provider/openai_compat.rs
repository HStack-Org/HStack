use super::{Message, ProviderConfig, Tool, Role};
use crate::error::Error;
use serde::{Deserialize, Serialize};
use tracing::{debug, enabled, trace, Level};


#[derive(Serialize)]
struct OpenAiMessage<'a> {
    role: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<&'a str>,
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

    let response_data_result: Result<OpenAiChatResponse, reqwest::Error> = response.json().await;
    let response_data = match response_data_result {
        Ok(data) => data,
        Err(e) => {
            debug!(error = %e, "failed to parse OpenAI-compatible response body");
            return Err(Error::ProviderContract(format!("Malformed provider response: {e}")));
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
