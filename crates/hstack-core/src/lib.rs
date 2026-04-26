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

// Shared contract surface for public and private HStack components.
// Review docs/public-private-contract.md before expanding shared models or APIs.
pub mod provider;
pub mod chat;
pub mod execution;
pub mod filesystem;
pub mod filesystem_error;
pub mod ticket;
pub mod sync;
pub mod stack_snapshot;
pub mod agent_proposals;
pub mod settings;
pub mod error;
pub mod location_utils;
pub mod temporal_parser;
pub mod api_models;
pub mod integration;
pub mod voice;
pub mod virtual_fs;
