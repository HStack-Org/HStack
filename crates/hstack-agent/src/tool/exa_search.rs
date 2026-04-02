use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::action::{AgentAction, WorkingMemoryDelta};
use crate::error::Error;
use crate::memory::HStackWorld;
use crate::tool::Tool;

/// Web search tool powered by Exa API.
pub struct ExaSearchTool {
    client: Client,
}

impl ExaSearchTool {
    pub fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

impl Default for ExaSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for ExaSearchTool {
    fn name(&self) -> &str {
        "exa_search"
    }

    fn description(&self) -> &str {
        "Runs a web search with Exa and stores concise results into working memory."
    }

    fn parameters(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query text." },
                "num_results": {
                    "type": "integer",
                    "description": "Number of results to return (1-25).",
                    "minimum": 1,
                    "maximum": 25
                },
                "include_domains": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional domain allowlist, for example ['docs.rs', 'github.com']."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, args: Value, _world: &dyn HStackWorld) -> Result<AgentAction, Error> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Internal("exa_search requires a non-empty 'query'".to_string()))?
            .to_string();

        let api_key = std::env::var("EXA_API_KEY")
            .map_err(|_| Error::Internal("EXA_API_KEY is not set".to_string()))?;
        let api_url = std::env::var("EXA_API_URL")
            .unwrap_or_else(|_| "https://api.exa.ai/search".to_string());

        let num_results = args
            .get("num_results")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(1, 25);

        let include_domains: Vec<String> = args
            .get("include_domains")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();

        let mut body = serde_json::json!({
            "query": query,
            "numResults": num_results
        });
        if !include_domains.is_empty() {
            body["includeDomains"] = serde_json::json!(include_domains);
        }

        let resp = self
            .client
            .post(api_url)
            .header("x-api-key", api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Network(e.to_string()))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "failed to read Exa error body".to_string());
            return Err(Error::Api {
                status: status.as_u16(),
                body,
            });
        }

        let payload: Value = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("Failed to parse Exa response: {e}")))?;

        let concise_results: Vec<Value> = payload
            .get("results")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        serde_json::json!({
                            "title": item.get("title").cloned().unwrap_or(Value::Null),
                            "url": item.get("url").cloned().unwrap_or(Value::Null),
                            "published_date": item.get("publishedDate").cloned().unwrap_or(Value::Null),
                            "score": item.get("score").cloned().unwrap_or(Value::Null),
                            "text": item.get("text").cloned().unwrap_or(Value::Null)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(AgentAction::UpdateWorkingMemory(
            WorkingMemoryDelta::AddTechnicalNoise(
                format!("exa_search:{query}"),
                serde_json::json!({
                    "query": query,
                    "num_results": num_results,
                    "results": concise_results
                }),
            ),
        ))
    }
}
