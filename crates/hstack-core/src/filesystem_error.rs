use crate::virtual_fs::VirtualPath;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Error)]
pub enum FilesystemError {
    #[error("path escapes above root")]
    PathEscapeAboveRoot,

    #[error("invalid virtual path: {reason}")]
    InvalidVirtualPath { reason: String },

    #[error("forbidden object kind at {path}")]
    ForbiddenObjectKind { path: VirtualPath },

    #[error("forbidden path class at {path}: {class}")]
    ForbiddenPathClass { path: VirtualPath, class: String },

    #[error("forbidden artifact class at {path}: {class}")]
    ForbiddenArtifactClass { path: VirtualPath, class: String },

    #[error("operation denied by policy: {reason}")]
    PolicyDenied { reason: String },

    #[error("conflict at {path}")]
    Conflict { path: VirtualPath },

    #[error("unsupported operation: {operation}")]
    UnsupportedOperation { operation: String },

    #[error("backend invariant violation: {reason}")]
    BackendInvariantViolation { reason: String },

    #[error("microbash parse error: {reason}")]
    ParseError { reason: String },

    #[error("microbash lowering error: {reason}")]
    LoweringError { reason: String },

    #[error("execution denied: {reason}")]
    ExecutionDenied { reason: String },

    #[error("execution failed: {reason}")]
    ExecutionFailed { reason: String },
}

#[cfg(test)]
mod tests {
    use super::FilesystemError;
    use crate::virtual_fs::VirtualPath;

    #[test]
    fn error_displays_path_sensitive_variants() {
        let path = VirtualPath::from_absolute("/src/main.rs")
            .unwrap_or_else(|e| panic!("path parse failed: {e}"));
        let error = FilesystemError::ForbiddenPathClass {
            path,
            class: "launch_agent".to_string(),
        };

        let rendered = error.to_string();
        assert!(rendered.contains("/src/main.rs"));
        assert!(rendered.contains("launch_agent"));
    }
}