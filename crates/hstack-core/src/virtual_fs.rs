use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VirtualPath(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum VirtualPathError {
    #[error("virtual path must not be empty")]
    Empty,

    #[error("virtual path must not contain NUL")]
    ContainsNul,

    #[error("virtual path must not use backslash separators")]
    BackslashSeparator,

    #[error("virtual path must not use host home expansion")]
    HomeExpansion,

    #[error("virtual path must not use authority-style prefixes")]
    AuthorityPrefix,

    #[error("virtual path must not use drive-letter prefixes")]
    DrivePrefix,

    #[error("virtual path cannot traverse above root")]
    EscapeAboveRoot,

    #[error("virtual path segment '{segment}' is not backend-neutral")]
    NonPortableSegment { segment: String },
}

impl VirtualPath {
    pub fn root() -> Self {
        Self("/".to_string())
    }

    pub fn from_absolute(input: &str) -> Result<Self, VirtualPathError> {
        if !input.starts_with('/') {
            return Err(VirtualPathError::DrivePrefix);
        }
        Self::resolve(&Self::root(), input)
    }

    pub fn resolve(cwd: &Self, input: &str) -> Result<Self, VirtualPathError> {
        validate_input(input)?;

        let mut segments = if input.starts_with('/') {
            Vec::new()
        } else {
            cwd.segments()
        };

        for segment in input.split('/') {
            match segment {
                "" | "." => {}
                ".." => {
                    if segments.pop().is_none() {
                        return Err(VirtualPathError::EscapeAboveRoot);
                    }
                }
                ordinary => {
                    validate_segment(ordinary)?;
                    segments.push(ordinary.to_string());
                }
            }
        }

        if segments.is_empty() {
            return Ok(Self::root());
        }

        Ok(Self(format!("/{}", segments.join("/"))))
    }

    pub fn join(&self, input: &str) -> Result<Self, VirtualPathError> {
        Self::resolve(self, input)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0 == "/"
    }

    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }

        let mut segments = self.segments();
        let _ = segments.pop();
        if segments.is_empty() {
            Some(Self::root())
        } else {
            Some(Self(format!("/{}", segments.join("/"))))
        }
    }

    pub fn file_name(&self) -> Option<&str> {
        if self.is_root() {
            return None;
        }
        self.0.rsplit('/').next()
    }

    pub fn segments(&self) -> Vec<String> {
        if self.is_root() {
            return Vec::new();
        }

        self.0
            .trim_start_matches('/')
            .split('/')
            .map(str::to_string)
            .collect()
    }
}

impl fmt::Display for VirtualPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn validate_input(input: &str) -> Result<(), VirtualPathError> {
    if input.is_empty() {
        return Err(VirtualPathError::Empty);
    }
    if input.contains('\0') {
        return Err(VirtualPathError::ContainsNul);
    }
    if input.contains('\\') {
        return Err(VirtualPathError::BackslashSeparator);
    }
    if input.starts_with('~') {
        return Err(VirtualPathError::HomeExpansion);
    }
    if input.starts_with("//") {
        return Err(VirtualPathError::AuthorityPrefix);
    }

    let bytes = input.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(VirtualPathError::DrivePrefix);
    }

    Ok(())
}

fn validate_segment(segment: &str) -> Result<(), VirtualPathError> {
    if segment.ends_with('.') || segment.ends_with(' ') {
        return Err(VirtualPathError::NonPortableSegment {
            segment: segment.to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{VirtualPath, VirtualPathError};

    #[test]
    fn resolves_relative_paths_inside_root() {
        let cwd = VirtualPath::from_absolute("/src/app").unwrap_or_else(|e| panic!("cwd parse failed: {e}"));
        let resolved = VirtualPath::resolve(&cwd, "../README.md")
            .unwrap_or_else(|e| panic!("relative resolve failed: {e}"));

        assert_eq!(resolved.as_str(), "/src/README.md");
    }

    #[test]
    fn normalizes_dot_and_empty_segments() {
        let resolved = VirtualPath::from_absolute("/src//./lib.rs")
            .unwrap_or_else(|e| panic!("absolute normalization failed: {e}"));

        assert_eq!(resolved.as_str(), "/src/lib.rs");
    }

    #[test]
    fn rejects_traversal_above_root() {
        let cwd = VirtualPath::root();
        let err = VirtualPath::resolve(&cwd, "../../etc/passwd")
            .err()
            .unwrap_or_else(|| panic!("expected traversal rejection"));

        assert_eq!(err, VirtualPathError::EscapeAboveRoot);
    }

    #[test]
    fn rejects_drive_letter_prefixes() {
        let err = VirtualPath::resolve(&VirtualPath::root(), "C:/temp/file.txt")
            .err()
            .unwrap_or_else(|| panic!("expected drive-prefix rejection"));

        assert_eq!(err, VirtualPathError::DrivePrefix);
    }

    #[test]
    fn rejects_home_expansion() {
        let err = VirtualPath::resolve(&VirtualPath::root(), "~/notes/today.md")
            .err()
            .unwrap_or_else(|| panic!("expected home-expansion rejection"));

        assert_eq!(err, VirtualPathError::HomeExpansion);
    }

    #[test]
    fn rejects_backslash_separators() {
        let err = VirtualPath::resolve(&VirtualPath::root(), "src\\main.rs")
            .err()
            .unwrap_or_else(|| panic!("expected backslash rejection"));

        assert_eq!(err, VirtualPathError::BackslashSeparator);
    }

    #[test]
    fn rejects_non_portable_trailing_segment_characters() {
        let err = VirtualPath::resolve(&VirtualPath::root(), "/src/trailing. ")
            .err()
            .unwrap_or_else(|| panic!("expected non-portable segment rejection"));

        assert_eq!(
            err,
            VirtualPathError::NonPortableSegment {
                segment: "trailing. ".to_string(),
            }
        );
    }

    #[test]
    fn parent_of_root_is_none() {
        assert_eq!(VirtualPath::root().parent(), None);
    }

    #[test]
    fn parent_of_nested_path_is_canonical() {
        let path = VirtualPath::from_absolute("/a/b/c.txt")
            .unwrap_or_else(|e| panic!("absolute parse failed: {e}"));

        let parent = path.parent().unwrap_or_else(|| panic!("expected parent path"));
        assert_eq!(parent.as_str(), "/a/b");
    }
}