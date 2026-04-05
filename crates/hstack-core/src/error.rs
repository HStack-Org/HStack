use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Network error: {0}")]
    Network(String),

    #[error("API error (status {status}): {body}")]
    Api {
        status: u16,
        body: String,
    },

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Header error: {0}")]
    Header(String),

    #[error("Provider error: {0}")]
    Provider(String),

    #[error("Provider contract error: {0}")]
    ProviderContract(String),

    #[error("Invariant violation: {0}")]
    Invariant(String),

    #[error("Rate limit exceeded")]
    RateLimit,

    #[error("Max tool iterations reached")]
    MaxIterations,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("Unknown error")]
    Unknown,
}
