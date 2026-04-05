use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::{AppId, SearchResultRecord, WorkspaceDelta};

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
        "Runs external web search with Exa for public web facts, docs, and websites. Not for local HStack content."
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

    async fn execute(&self, args: Value, _world: &dyn HStackWorld, _memory: &WorkingMemory) -> Result<AgentAction, Error> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Provider("exa_search requires a non-empty 'query'".to_string()))?
            .to_string();

        let num_results = args
            .get("num_results")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| Error::Provider("exa_search 'num_results' must be an integer".to_string()))
            })
            .transpose()?
            .unwrap_or(5);
        if !(1..=25).contains(&num_results) {
            return Err(Error::Provider(
                "exa_search 'num_results' must be between 1 and 25".to_string(),
            ));
        }

        let include_domains: Vec<String> = match args.get("include_domains") {
            None => Vec::new(),
            Some(Value::Array(arr)) => {
                let mut domains = Vec::with_capacity(arr.len());
                for item in arr {
                    let domain = item
                        .as_str()
                        .map(str::trim)
                        .filter(|domain| !domain.is_empty())
                        .ok_or_else(|| {
                            Error::Provider(
                                "exa_search 'include_domains' must contain non-empty strings".to_string(),
                            )
                        })?;
                    domains.push(domain.to_string());
                }
                domains
            }
            Some(_) => {
                return Err(Error::Provider(
                    "exa_search 'include_domains' must be an array of strings".to_string(),
                ))
            }
        };

        let api_key = std::env::var("EXA_API_KEY")
            .map_err(|_| Error::Configuration("EXA_API_KEY is not set".to_string()))?;
        let api_url = std::env::var("EXA_API_URL")
            .unwrap_or_else(|_| "https://api.exa.ai/search".to_string());

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

        let concise_results: Vec<SearchResultRecord> = payload
            .get("results")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| {
                        SearchResultRecord {
                            title: item
                                .get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("Untitled")
                                .to_string(),
                            url: item.get("url").and_then(Value::as_str).map(str::to_string),
                            snippet: item.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                            metadata: serde_json::json!({
                                "published_date": item.get("publishedDate").cloned().unwrap_or(Value::Null),
                                "score": item.get("score").cloned().unwrap_or(Value::Null)
                            }),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(AgentAction::UpdateWorkspace(WorkspaceDelta::PublishSearchResults {
            app_id: AppId::WebSearch,
            query,
            results: concise_results,
        }))
    }
}
