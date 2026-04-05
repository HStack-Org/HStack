#![deny(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(
	not(test),
	deny(
		clippy::panic,
		clippy::panic_in_result_fn,
		clippy::todo,
		clippy::unimplemented,
		clippy::unreachable
	)
)]

pub mod action;
pub mod agent;
pub mod control;
pub mod error;
pub mod formal;
pub mod manager;
pub mod memory;
pub mod prompt;
pub mod provider;
pub mod rate_limiter;
pub mod tool;
pub mod workspace;
mod tests;

pub use action::{AgentAction, DecodeAnomaly, DecodeAnomalyKind, DecodedTurn};
pub use agent::{Agent, AgentProgressUpdate};
pub use control::AgentControlSystem;
pub use error::Error;
pub use manager::ContextManager;
pub use memory::{HStackWorld, WorkingMemory};
pub use prompt::{build_base_prompt, AgentPromptProfile};
pub use rate_limiter::{RateLimiter, RateLimitConfig, LocalRateLimiter, RedisRateLimiter};
pub use tool::Tool;
pub use workspace::{AppId, AppLifecycle, ContextBudget, WorkspaceDelta, WorkspaceState};
