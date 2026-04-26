use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

use crate::action::AgentAction;
use crate::error::Error;
use crate::memory::{HStackWorld, WorkingMemory};
use crate::tool::Tool;
use crate::workspace::{AppId, SearchResultRecord, WorkspaceDelta};

const DEFAULT_WEB_SEARCH_PROVIDER: &str = "exa";

#[derive(Debug, Clone)]
struct WebSearchRequest {
    query: String,
    num_results: u64,
    include_domains: Vec<String>,
}

#[async_trait]
trait WebSearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn is_available(&self) -> bool;
    async fn search(&self, request: &WebSearchRequest) -> Result<Vec<SearchResultRecord>, Error>;
}

struct ExaWebSearchProvider {
    client: Client,
}

impl ExaWebSearchProvider {
    fn new() -> Self {
        Self {
            client: Client::new(),
        }
    }
}

#[async_trait]
impl WebSearchProvider for ExaWebSearchProvider {
    fn name(&self) -> &'static str {
        "exa"
    }

    fn is_available(&self) -> bool {
        std::env::var("EXA_API_KEY")
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    }

    async fn search(&self, request: &WebSearchRequest) -> Result<Vec<SearchResultRecord>, Error> {
        let api_key = std::env::var("EXA_API_KEY")
            .map_err(|_| Error::Configuration("EXA_API_KEY is not set".to_string()))?;
        let api_url = std::env::var("EXA_API_URL")
            .unwrap_or_else(|_| "https://api.exa.ai/search".to_string());

        let mut body = serde_json::json!({
            "query": request.query,
            "numResults": request.num_results,
        });
        if !request.include_domains.is_empty() {
            body["includeDomains"] = serde_json::json!(request.include_domains);
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
                .unwrap_or_else(|_| "failed to read web search provider error body".to_string());
            return Err(Error::Api {
                status: status.as_u16(),
                body,
            });
        }

        let payload: Value = resp
            .json()
            .await
            .map_err(|e| Error::Provider(format!("Failed to parse Exa response: {e}")))?;

        Ok(payload
            .get("results")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .map(|item| SearchResultRecord {
                        title: item
                            .get("title")
                            .and_then(Value::as_str)
                            .unwrap_or("Untitled")
                            .to_string(),
                        url: item.get("url").and_then(Value::as_str).map(str::to_string),
                        snippet: item.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
                        metadata: serde_json::json!({
                            "provider": self.name(),
                            "published_date": item.get("publishedDate").cloned().unwrap_or(Value::Null),
                            "score": item.get("score").cloned().unwrap_or(Value::Null)
                        }),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }
}

fn configured_web_search_provider_name() -> String {
    std::env::var("HSTACK_WEB_SEARCH_PROVIDER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| DEFAULT_WEB_SEARCH_PROVIDER.to_string())
}

fn build_default_web_search_provider() -> Result<Box<dyn WebSearchProvider>, Error> {
    match configured_web_search_provider_name().as_str() {
        "exa" => Ok(Box::new(ExaWebSearchProvider::new())),
        other => Err(Error::Configuration(format!(
            "Unsupported web search provider '{other}'. Supported providers: exa"
        ))),
    }
}

pub fn web_search_is_available() -> bool {
    match build_default_web_search_provider() {
        Ok(provider) => provider.is_available(),
        Err(_) => false,
    }
}

/// External web search tool backed by a configurable provider. Exa is the default provider.
pub struct WebSearchTool {
    provider: Box<dyn WebSearchProvider>,
}

impl WebSearchTool {
    pub fn new() -> Result<Self, Error> {
        Ok(Self {
            provider: build_default_web_search_provider()?,
        })
    }

    fn parse_request(args: Value) -> Result<WebSearchRequest, Error> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::Provider("web_search requires a non-empty 'query'".to_string()))?
            .to_string();

        let num_results = args
            .get("num_results")
            .map(|value| {
                value
                    .as_u64()
                    .ok_or_else(|| Error::Provider("web_search 'num_results' must be an integer".to_string()))
            })
            .transpose()?
            .unwrap_or(5);
        if !(1..=25).contains(&num_results) {
            return Err(Error::Provider(
                "web_search 'num_results' must be between 1 and 25".to_string(),
            ));
        }

        let include_domains = match args.get("include_domains") {
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
                                "web_search 'include_domains' must contain non-empty strings".to_string(),
                            )
                        })?;
                    domains.push(domain.to_string());
                }
                domains
            }
            Some(_) => {
                return Err(Error::Provider(
                    "web_search 'include_domains' must be an array of strings".to_string(),
                ))
            }
        };

        Ok(WebSearchRequest {
            query,
            num_results,
            include_domains,
        })
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        match Self::new() {
            Ok(tool) => tool,
            Err(e) => panic!("failed to initialize web_search tool: {e}"),
        }
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Runs external web search with the configured provider (Exa by default) for public web facts, docs, and websites. Not for local HStack content."
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
        let request = Self::parse_request(args)?;
        let query = request.query.clone();
        let results = self.provider.search(&request).await?;

        Ok(AgentAction::UpdateWorkspace(WorkspaceDelta::PublishSearchResults {
            app_id: AppId::WebSearch,
            query,
            results,
        }))
    }
}