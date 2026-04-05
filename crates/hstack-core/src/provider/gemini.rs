use super::{Message, ProviderConfig, Role, Tool, ToolCall, ToolFunctionCall};
use crate::error::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SKIP_THOUGHT_SIGNATURE_VALIDATOR: &str = "skip_thought_signature_validator";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<GeminiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiContent {
    role: String,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct GeminiPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_call: Option<GeminiFunctionCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    function_response: Option<GeminiFunctionResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thought_signature: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiFunctionCall {
    name: String,
    args: Value,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct GeminiFunctionResponse {
    name: String,
    response: Value,
}

#[derive(Serialize)]
struct GeminiTool {
    function_declarations: Vec<GeminiFunctionDeclaration>,
}

#[derive(Serialize)]
struct GeminiFunctionDeclaration {
    name: String,
    description: String,
    parameters: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GeminiResponse {
    candidates: Option<Vec<GeminiCandidate>>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiContent,
}

pub async fn generate_gemini_content(
    config: &ProviderConfig,
    messages: &[Message],
    tools: Option<&[Tool]>,
) -> Result<Message, Error> {
    let client = reqwest::Client::new();
    let api_url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        config.model_name, config.api_key
    );

    let mut gemini_contents = Vec::new();
    let mut system_instruction = None;

    for msg in messages {
        match msg.role {
            Role::System => {
                let part = GeminiPart {
                    text: msg.content.clone(),
                    function_call: None,
                    function_response: None,
                    thought_signature: None,
                };
                system_instruction = Some(GeminiContent {
                    role: "user".to_string(),
                    parts: vec![part],
                });
            }
            Role::User => {
                let part = GeminiPart {
                    text: msg.content.clone(),
                    function_call: None,
                    function_response: None,
                    thought_signature: None,
                };
                gemini_contents.push(GeminiContent {
                    role: "user".to_string(),
                    parts: vec![part],
                });
            }
            Role::Assistant => {
                let mut parts = Vec::new();
                if let Some(text) = &msg.content {
                    parts.push(GeminiPart {
                        text: Some(text.clone()),
                        function_call: None,
                        function_response: None,
                        thought_signature: None,
                    });
                }
                if let Some(calls) = &msg.tool_calls {
                    for (index, call) in calls.iter().enumerate() {
                        let args_val_result = serde_json::from_str::<Value>(&call.function.arguments);
                        let args_val = match args_val_result {
                            Ok(v) => v,
                            Err(e) => {
                                return Err(Error::Invariant(format!(
                                    "assistant tool call '{}' carried malformed stored JSON arguments: {e}",
                                    call.function.name
                                )))
                            }
                        };
                        parts.push(GeminiPart {
                            text: None,
                            function_call: Some(GeminiFunctionCall {
                                name: call.function.name.clone(),
                                args: args_val,
                            }),
                            function_response: None,
                            thought_signature: if index == 0 {
                                Some(SKIP_THOUGHT_SIGNATURE_VALIDATOR.to_string())
                            } else {
                                None
                            },
                        });
                    }
                }
                gemini_contents.push(GeminiContent {
                    role: "model".to_string(),
                    parts,
                });
            }
            Role::Tool => {
                let response_val = match &msg.content {
                    Some(content) => match serde_json::from_str::<Value>(content) {
                        Ok(v) => v,
                        Err(_) => serde_json::json!({ "result": content }),
                    },
                    None => {
                        return Err(Error::Invariant(
                            "tool message is missing content for Gemini provider encoding".to_string(),
                        ))
                    }
                };
                let tool_name = msg.name.clone().ok_or_else(|| {
                    Error::Invariant(
                        "tool message is missing name for Gemini provider encoding".to_string(),
                    )
                })?;

                let part = GeminiPart {
                    text: None,
                    function_call: None,
                    function_response: Some(GeminiFunctionResponse {
                        name: tool_name,
                        response: response_val,
                    }),
                    thought_signature: None,
                };
                gemini_contents.push(GeminiContent {
                    role: "user".to_string(),
                    parts: vec![part],
                });
            }
        }
    }

    let gemini_tools = match tools {
        Some(t_list) => {
            let declarations = t_list
                .iter()
                .map(|t| GeminiFunctionDeclaration {
                    name: t.function.name.clone(),
                    description: t.function.description.clone(),
                    parameters: t.function.parameters.clone(),
                })
                .collect::<Vec<_>>();
            Some(vec![GeminiTool {
                function_declarations: declarations,
            }])
        }
        None => None,
    };

    let request = GeminiRequest {
        contents: gemini_contents,
        tools: gemini_tools,
        system_instruction,
    };

    let response_result = client
        .post(&api_url)
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
        return Err(Error::Api { status, body });
    }

    let response_data_result: Result<GeminiResponse, reqwest::Error> = response.json().await;
    let response_data = match response_data_result {
        Ok(data) => data,
        Err(e) => return Err(Error::ProviderContract(format!("Malformed provider response: {e}"))),
    };

    let candidates = match response_data.candidates {
        Some(c) => c,
        None => return Err(Error::ProviderContract("API returned no candidates".to_string())),
    };

    let mut candidate_iter = candidates.into_iter();
    let candidate = match candidate_iter.next() {
        Some(c) => c,
        None => return Err(Error::ProviderContract("API returned empty candidates list".to_string())),
    };

    let mut content_str = None;
    let mut tool_calls = Vec::new();

    for part in candidate.content.parts {
        if let Some(text) = part.text {
            content_str = Some(text);
        }
        if let Some(fc) = part.function_call {
            let args_string = serde_json::to_string(&fc.args)
                .map_err(|e| Error::ProviderContract(format!("Gemini function_call args were not serializable JSON: {e}")))?;
            tool_calls.push(ToolCall {
                id: "gemini_call".to_string(),
                r#type: "function".to_string(),
                function: ToolFunctionCall {
                    name: fc.name,
                    arguments: args_string,
                },
            });
        }
    }

    Ok(Message {
        role: Role::Assistant,
        content: content_str,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        tool_call_id: None,
        name: None,
    })
}
