use hstack_core::filesystem::{
    CapabilityProfile, ConflictToken, DeleteMode, DeleteResult, DirectoryEntry, FileStat,
    FilesystemInstruction, FilesystemOutcome, FilesystemPolicy, FsObjectKind, ListingOrder,
    MoveResult, ReadResult, SearchMatch, SearchScope, WriteMode, WriteResult,
};
use hstack_core::virtual_fs::VirtualPath;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::error::Error;

pub const SANDBOX_ROOT_ENV: &str = "HSTACK_AGENT_SANDBOX_ROOT";
pub const SANDBOX_PROFILE_ENV: &str = "HSTACK_AGENT_SANDBOX_PROFILE";

pub struct LocalSandboxedFilesystem {
    root: PathBuf,
    policy: FilesystemPolicy,
}

impl LocalSandboxedFilesystem {
    pub fn new(root: PathBuf, policy: FilesystemPolicy) -> Result<Self, Error> {
        validate_local_root(&root)?;
        fs::create_dir_all(&root)
            .map_err(|e| Error::Sandbox(format!("failed to create sandbox root '{}': {e}", root.display())))?;

        Ok(Self { root, policy })
    }

    pub fn execute(&self, instruction: &FilesystemInstruction) -> Result<FilesystemOutcome, Error> {
        match instruction {
            FilesystemInstruction::ListDir { path, limit } => {
                self.list_dir(path, *limit).map(FilesystemOutcome::ListDir)
            }
            FilesystemInstruction::Stat { path } => self.stat(path).map(FilesystemOutcome::Stat),
            FilesystemInstruction::ReadFile { path, offset, limit } => self
                .read_file(path, *offset, *limit)
                .map(FilesystemOutcome::ReadFile),
            FilesystemInstruction::WriteFile {
                path,
                content,
                mode,
                expected_conflict_token,
            } => self
                .write_file(path, content, *mode, expected_conflict_token.as_ref())
                .map(FilesystemOutcome::WriteFile),
            FilesystemInstruction::PatchFile {
                path,
                patch,
                expected_conflict_token,
            } => self
                .patch_file(path, patch, expected_conflict_token.as_ref())
                .map(FilesystemOutcome::PatchFile),
            FilesystemInstruction::CreateDir { path, recursive } => {
                self.create_dir(path, *recursive)?;
                Ok(FilesystemOutcome::CreateDir)
            }
            FilesystemInstruction::MovePath {
                from,
                to,
                overwrite,
            } => self
                .move_path(from, to, *overwrite)
                .map(FilesystemOutcome::MovePath),
            FilesystemInstruction::DeletePath { path, mode } => self
                .delete_path(path, *mode)
                .map(FilesystemOutcome::DeletePath),
            FilesystemInstruction::SearchText { scope, query, limit } => self
                .search_text(scope, query, *limit)
                .map(FilesystemOutcome::SearchText),
        }
    }

    pub fn stat(&self, path: &VirtualPath) -> Result<FileStat, Error> {
        self.ensure_readable(path)?;
        let real = self.resolve_existing_path(path)?;
        let metadata = fs::metadata(&real)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", real.display())))?;

        Ok(FileStat {
            path: path.clone(),
            kind: metadata_kind(&metadata)?,
            size_bytes: if metadata.is_file() { Some(metadata.len()) } else { None },
            conflict_token: Some(conflict_token_for_metadata(&metadata)?),
        })
    }

    pub fn list_dir(&self, path: &VirtualPath, limit: Option<u64>) -> Result<Vec<DirectoryEntry>, Error> {
        self.ensure_readable(path)?;
        let real = self.resolve_existing_path(path)?;
        let metadata = fs::metadata(&real)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", real.display())))?;
        if !metadata.is_dir() {
            return Err(Error::Sandbox(format!("'{}' is not a directory", path)));
        }

        let effective_limit = limit.unwrap_or(self.policy.max_directory_entries).min(self.policy.max_directory_entries);
        let mut entries = Vec::new();

        for entry in fs::read_dir(&real)
            .map_err(|e| Error::Sandbox(format!("failed to list '{}': {e}", real.display())))?
        {
            let entry = entry.map_err(|e| Error::Sandbox(format!("failed to read directory entry: {e}")))?;
            let child_real = entry.path();
            reject_forbidden_real_path(&child_real)?;

            let metadata = fs::metadata(&child_real)
                .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", child_real.display())))?;
            let name = entry.file_name().to_string_lossy().to_string();
            let child_virtual = path.join(&name).map_err(|e| Error::Sandbox(format!("virtual child path error: {e}")))?;
            entries.push(DirectoryEntry {
                path: child_virtual,
                name,
                kind: metadata_kind(&metadata)?,
                conflict_token: Some(conflict_token_for_metadata(&metadata)?),
            });
        }

        if self.policy.listing_order == ListingOrder::Lexicographic {
            entries.sort_by(|left, right| left.name.cmp(&right.name));
        }
        entries.truncate(effective_limit as usize);
        Ok(entries)
    }

    pub fn read_file(&self, path: &VirtualPath, offset: u64, limit: u64) -> Result<ReadResult, Error> {
        self.ensure_readable(path)?;
        if limit > self.policy.max_read_bytes {
            return Err(Error::Sandbox(format!(
                "read limit {} exceeds policy max {}",
                limit, self.policy.max_read_bytes
            )));
        }

        let real = self.resolve_existing_path(path)?;
        let metadata = fs::metadata(&real)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", real.display())))?;
        if !metadata.is_file() {
            return Err(Error::Sandbox(format!("'{}' is not a regular file", path)));
        }

        let bytes = fs::read(&real)
            .map_err(|e| Error::Sandbox(format!("failed to read '{}': {e}", real.display())))?;
        let start = usize::try_from(offset).map_err(|_| Error::Sandbox("read offset too large".to_string()))?;
        let end = start.saturating_add(limit as usize).min(bytes.len());
        let content = if start >= bytes.len() {
            Vec::new()
        } else {
            bytes[start..end].to_vec()
        };

        Ok(ReadResult {
            path: path.clone(),
            offset,
            limit,
            content,
            conflict_token: Some(conflict_token_for_metadata(&metadata)?),
        })
    }

    pub fn write_file(
        &self,
        path: &VirtualPath,
        content: &[u8],
        mode: WriteMode,
        expected_conflict_token: Option<&ConflictToken>,
    ) -> Result<WriteResult, Error> {
        self.ensure_writable(path)?;
        if (content.len() as u64) > self.policy.max_write_bytes {
            return Err(Error::Sandbox(format!(
                "write size {} exceeds policy max {}",
                content.len(), self.policy.max_write_bytes
            )));
        }

        let real = self.resolve_path(path)?;
        let parent = real.parent().ok_or_else(|| Error::Sandbox("target path has no parent".to_string()))?;
        fs::create_dir_all(parent)
            .map_err(|e| Error::Sandbox(format!("failed to create parent directories '{}': {e}", parent.display())))?;

        let exists = real.exists();
        if exists {
            reject_forbidden_real_path(&real)?;
            let metadata = fs::metadata(&real)
                .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", real.display())))?;
            if !metadata.is_file() {
                return Err(Error::Sandbox(format!("'{}' is not a regular file", path)));
            }
            if let Some(expected) = expected_conflict_token {
                let actual = conflict_token_for_metadata(&metadata)?;
                if &actual != expected {
                    return Err(Error::Sandbox(format!("conflict token mismatch for '{}'", path)));
                }
            }
        }

        match mode {
            WriteMode::CreateOnly if exists => {
                return Err(Error::Sandbox(format!("'{}' already exists", path)));
            }
            WriteMode::ReplaceIfTokenMatches if expected_conflict_token.is_none() => {
                return Err(Error::Sandbox(format!(
                    "replace-if-token-matches requires a conflict token for '{}'",
                    path
                )));
            }
            WriteMode::CreateOnly | WriteMode::Truncate | WriteMode::Replace | WriteMode::ReplaceIfTokenMatches => {}
        }

        fs::write(&real, content)
            .map_err(|e| Error::Sandbox(format!("failed to write '{}': {e}", real.display())))?;

        let metadata = fs::metadata(&real)
            .map_err(|e| Error::Sandbox(format!("failed to stat written file '{}': {e}", real.display())))?;
        Ok(WriteResult {
            path: path.clone(),
            bytes_written: content.len() as u64,
            conflict_token: Some(conflict_token_for_metadata(&metadata)?),
        })
    }

    pub fn create_dir(&self, path: &VirtualPath, recursive: bool) -> Result<(), Error> {
        self.ensure_creatable(path)?;
        let real = self.resolve_path(path)?;
        if recursive {
            fs::create_dir_all(&real)
                .map_err(|e| Error::Sandbox(format!("failed to create directory '{}': {e}", real.display())))?;
        } else {
            fs::create_dir(&real)
                .map_err(|e| Error::Sandbox(format!("failed to create directory '{}': {e}", real.display())))?;
        }
        Ok(())
    }

    pub fn patch_file(
        &self,
        path: &VirtualPath,
        patch: &str,
        expected_conflict_token: Option<&ConflictToken>,
    ) -> Result<WriteResult, Error> {
        self.ensure_writable(path)?;
        let real = self.resolve_existing_path(path)?;
        let metadata = fs::metadata(&real)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", real.display())))?;
        if !metadata.is_file() {
            return Err(Error::Sandbox(format!("'{}' is not a regular file", path)));
        }

        if let Some(expected) = expected_conflict_token {
            let actual = conflict_token_for_metadata(&metadata)?;
            if &actual != expected {
                return Err(Error::Sandbox(format!("conflict token mismatch for '{}'", path)));
            }
        }

        let patch_spec: TextPatchSpec = serde_json::from_str(patch)
            .map_err(|e| Error::Sandbox(format!("invalid patch payload for '{}': {e}", path)))?;
        let current = fs::read_to_string(&real)
            .map_err(|e| Error::Sandbox(format!("failed to read text file '{}': {e}", real.display())))?;
        let mut lines: Vec<String> = current.lines().map(str::to_string).collect();
        if current.ends_with('\n') {
            lines.push(String::new());
        }

        if patch_spec.start_line > lines.len() {
            return Err(Error::Sandbox(format!(
                "patch start_line {} exceeds file line count {} for '{}'",
                patch_spec.start_line,
                lines.len(),
                path
            )));
        }

        let delete_end = patch_spec.start_line.saturating_add(patch_spec.delete_count).min(lines.len());
        lines.splice(
            patch_spec.start_line..delete_end,
            patch_spec.new_lines.iter().cloned(),
        );

        let updated = if lines.last().is_some_and(String::is_empty) {
            let mut joined = lines[..lines.len().saturating_sub(1)].join("\n");
            joined.push('\n');
            joined
        } else {
            lines.join("\n")
        };

        self.write_file(
            path,
            updated.as_bytes(),
            WriteMode::Replace,
            expected_conflict_token,
        )
    }

    pub fn move_path(&self, from: &VirtualPath, to: &VirtualPath, overwrite: bool) -> Result<MoveResult, Error> {
        self.ensure_deletable(from)?;
        self.ensure_writable(to)?;

        let from_real = self.resolve_existing_path(from)?;
        let to_real = self.resolve_path(to)?;
        let overwritten = to_real.exists();
        if overwritten && !overwrite {
            return Err(Error::Sandbox(format!("destination '{}' already exists", to)));
        }
        if overwritten {
            if to_real.is_dir() {
                fs::remove_dir_all(&to_real)
                    .map_err(|e| Error::Sandbox(format!("failed to remove existing directory '{}': {e}", to_real.display())))?;
            } else {
                fs::remove_file(&to_real)
                    .map_err(|e| Error::Sandbox(format!("failed to remove existing file '{}': {e}", to_real.display())))?;
            }
        }
        if let Some(parent) = to_real.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::Sandbox(format!("failed to create destination parent '{}': {e}", parent.display())))?;
        }
        fs::rename(&from_real, &to_real)
            .map_err(|e| Error::Sandbox(format!("failed to move '{}' to '{}': {e}", from, to)))?;

        let metadata = fs::metadata(&to_real)
            .map_err(|e| Error::Sandbox(format!("failed to stat moved target '{}': {e}", to_real.display())))?;
        Ok(MoveResult {
            from: from.clone(),
            to: to.clone(),
            overwritten,
            conflict_token: Some(conflict_token_for_metadata(&metadata)?),
        })
    }

    pub fn delete_path(&self, path: &VirtualPath, mode: DeleteMode) -> Result<DeleteResult, Error> {
        self.ensure_deletable(path)?;
        let real = self.resolve_existing_path(path)?;
        let metadata = fs::metadata(&real)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", real.display())))?;

        let deleted_count = if metadata.is_dir() {
            match mode {
                DeleteMode::SinglePath => {
                    fs::remove_dir(&real)
                        .map_err(|e| Error::Sandbox(format!("failed to remove directory '{}': {e}", real.display())))?;
                    1
                }
                DeleteMode::Recursive => {
                    let count = count_entries(&real)?;
                    fs::remove_dir_all(&real)
                        .map_err(|e| Error::Sandbox(format!("failed to remove directory tree '{}': {e}", real.display())))?;
                    count
                }
            }
        } else {
            fs::remove_file(&real)
                .map_err(|e| Error::Sandbox(format!("failed to remove file '{}': {e}", real.display())))?;
            1
        };

        Ok(DeleteResult {
            path: path.clone(),
            deleted_count,
        })
    }

    pub fn search_text(&self, scope: &SearchScope, query: &str, limit: u64) -> Result<Vec<SearchMatch>, Error> {
        self.ensure_readable(&scope.root)?;
        let effective_limit = limit.min(self.policy.max_search_matches);
        let root = self.resolve_existing_path(&scope.root)?;
        let mut matches = Vec::new();
        collect_search_matches(&root, &scope.root, query, scope.recursive, effective_limit as usize, &mut matches)?;
        Ok(matches)
    }

    fn ensure_readable(&self, path: &VirtualPath) -> Result<(), Error> {
        ensure_in_roots(path, &self.policy.readable_roots, "read")?;
        ensure_not_forbidden(path, &self.policy)?;
        Ok(())
    }

    fn ensure_writable(&self, path: &VirtualPath) -> Result<(), Error> {
        ensure_in_roots(path, &self.policy.writable_roots, "write")?;
        ensure_not_forbidden(path, &self.policy)?;
        Ok(())
    }

    fn ensure_creatable(&self, path: &VirtualPath) -> Result<(), Error> {
        ensure_in_roots(path, &self.policy.creatable_roots, "create")?;
        ensure_not_forbidden(path, &self.policy)?;
        Ok(())
    }

    fn ensure_deletable(&self, path: &VirtualPath) -> Result<(), Error> {
        ensure_in_roots(path, &self.policy.deletable_roots, "delete")?;
        ensure_not_forbidden(path, &self.policy)?;
        Ok(())
    }

    fn resolve_existing_path(&self, path: &VirtualPath) -> Result<PathBuf, Error> {
        let real = self.resolve_path(path)?;
        if !real.exists() {
            return Err(Error::Sandbox(format!("'{}' does not exist", path)));
        }
        reject_forbidden_real_path(&real)?;
        Ok(real)
    }

    fn resolve_path(&self, path: &VirtualPath) -> Result<PathBuf, Error> {
        let mut real = self.root.clone();
        for segment in path.segments() {
            real.push(segment);
        }

        let mut cursor = self.root.clone();
        for component in real.strip_prefix(&self.root)
            .map_err(|e| Error::Sandbox(format!("failed to strip sandbox root: {e}")))?
            .components()
        {
            if let Component::Normal(part) = component {
                cursor.push(part);
                if cursor.exists() {
                    reject_forbidden_real_path(&cursor)?;
                }
            }
        }

        Ok(real)
    }
}

pub fn filesystem_is_configured() -> bool {
    std::env::var_os(SANDBOX_ROOT_ENV).is_some()
}

pub fn configured_local_filesystem() -> Result<LocalSandboxedFilesystem, Error> {
    let root = std::env::var(SANDBOX_ROOT_ENV).map_err(|_| {
        Error::Configuration(format!("{SANDBOX_ROOT_ENV} is not set"))
    })?;
    let profile = configured_capability_profile()?;
    let policy = match profile {
        CapabilityProfile::SafeDataSandbox => FilesystemPolicy::safe_data_sandbox(VirtualPath::root()),
        CapabilityProfile::ProjectSandbox => FilesystemPolicy::project_sandbox(VirtualPath::root()),
    };

    LocalSandboxedFilesystem::new(PathBuf::from(root), policy)
}

fn configured_capability_profile() -> Result<CapabilityProfile, Error> {
    let raw = std::env::var(SANDBOX_PROFILE_ENV).unwrap_or_else(|_| "project_sandbox".to_string());
    match raw.as_str() {
        "safe_data_sandbox" => Ok(CapabilityProfile::SafeDataSandbox),
        "project_sandbox" => Ok(CapabilityProfile::ProjectSandbox),
        other => Err(Error::Configuration(format!(
            "{SANDBOX_PROFILE_ENV} must be 'safe_data_sandbox' or 'project_sandbox', got '{other}'"
        ))),
    }
}

fn ensure_in_roots(path: &VirtualPath, roots: &[VirtualPath], action: &str) -> Result<(), Error> {
    if roots.iter().any(|root| path_starts_with(path, root)) {
        return Ok(());
    }

    Err(Error::Denied(format!(
        "filesystem path '{}' is not permitted for {}",
        path, action
    )))
}

fn ensure_not_forbidden(path: &VirtualPath, policy: &FilesystemPolicy) -> Result<(), Error> {
    let path_str = path.as_str();
    for forbidden in &policy.forbidden_path_classes {
        if path_str == forbidden || path_str.starts_with(&format!("{forbidden}/")) {
            return Err(Error::Denied(format!(
                "filesystem path '{}' matches forbidden path class '{}'",
                path, forbidden
            )));
        }
    }

    if let Some(name) = path.file_name() {
        for forbidden in &policy.forbidden_artifact_classes {
            if artifact_class_matches(name, forbidden) {
                return Err(Error::Denied(format!(
                    "filesystem path '{}' matches forbidden artifact class '{}'",
                    path, forbidden
                )));
            }
        }
    }

    Ok(())
}

fn artifact_class_matches(name: &str, forbidden: &str) -> bool {
    if let Some(suffix) = forbidden.strip_prefix("*.") {
        return name
            .rsplit_once('.')
            .map(|(_, ext)| ext.eq_ignore_ascii_case(suffix))
            .unwrap_or(false);
    }

    name == forbidden
}

fn path_starts_with(path: &VirtualPath, root: &VirtualPath) -> bool {
    if root.is_root() {
        return true;
    }

    let path_segments = path.segments();
    let root_segments = root.segments();
    path_segments.starts_with(&root_segments)
}

fn metadata_kind(metadata: &fs::Metadata) -> Result<FsObjectKind, Error> {
    if metadata.is_file() {
        return Ok(FsObjectKind::RegularFile);
    }
    if metadata.is_dir() {
        return Ok(FsObjectKind::Directory);
    }
    Err(Error::Sandbox("forbidden filesystem object kind encountered".to_string()))
}

fn conflict_token_for_metadata(metadata: &fs::Metadata) -> Result<ConflictToken, Error> {
    let modified = metadata.modified()
        .map_err(|e| Error::Sandbox(format!("failed to read modification time: {e}")))?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Sandbox(format!("invalid modification time: {e}")))?
        .as_nanos();
    Ok(ConflictToken(format!("{}:{modified}", metadata.len())))
}

fn reject_forbidden_real_path(path: &Path) -> Result<(), Error> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| Error::Sandbox(format!("failed to inspect '{}': {e}", path.display())))?;
    let file_type = metadata.file_type();
    if file_type.is_symlink() {
        return Err(Error::Sandbox(format!("symlinks are forbidden: '{}'", path.display())));
    }
    if !(file_type.is_dir() || file_type.is_file()) {
        return Err(Error::Sandbox(format!(
            "forbidden filesystem object encountered: '{}'",
            path.display()
        )));
    }
    Ok(())
}

fn validate_local_root(root: &Path) -> Result<(), Error> {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if root == home {
            return Err(Error::Sandbox(format!(
                "sandbox root '{}' must not be the real user home directory",
                root.display()
            )));
        }
        let forbidden_under_home = [
            ".config/autostart",
            ".config/systemd/user",
            ".local/share/systemd/user",
            "Library/LaunchAgents",
            "Library/LoginItems",
        ];
        for suffix in forbidden_under_home {
            if root.starts_with(home.join(suffix)) {
                return Err(Error::Sandbox(format!(
                    "sandbox root '{}' must not map to ambient host activation paths",
                    root.display()
                )));
            }
        }
    }

    let forbidden_absolute_roots = [
        PathBuf::from("/Library/LaunchAgents"),
        PathBuf::from("/Library/LaunchDaemons"),
        PathBuf::from("/System/Library/LaunchAgents"),
        PathBuf::from("/System/Library/LaunchDaemons"),
        PathBuf::from("/etc/systemd/system"),
        PathBuf::from("/etc/xdg/autostart"),
    ];
    for forbidden in forbidden_absolute_roots {
        if root.starts_with(&forbidden) {
            return Err(Error::Sandbox(format!(
                "sandbox root '{}' must not map to ambient host activation paths",
                root.display()
            )));
        }
    }

    Ok(())
}

fn count_entries(path: &Path) -> Result<u64, Error> {
    let mut total = 1u64;
    for entry in fs::read_dir(path)
        .map_err(|e| Error::Sandbox(format!("failed to traverse '{}': {e}", path.display())))?
    {
        let entry = entry.map_err(|e| Error::Sandbox(format!("failed to read directory entry: {e}")))?;
        let child = entry.path();
        reject_forbidden_real_path(&child)?;
        let metadata = fs::metadata(&child)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", child.display())))?;
        if metadata.is_dir() {
            total = total.saturating_add(count_entries(&child)?);
        } else {
            total = total.saturating_add(1);
        }
    }
    Ok(total)
}

fn collect_search_matches(
    real_root: &Path,
    virtual_root: &VirtualPath,
    query: &str,
    recursive: bool,
    limit: usize,
    matches: &mut Vec<SearchMatch>,
) -> Result<(), Error> {
    let mut entries = fs::read_dir(real_root)
        .map_err(|e| Error::Sandbox(format!("failed to list '{}': {e}", real_root.display())))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| Error::Sandbox(format!("failed to read directory entry: {e}")))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if matches.len() >= limit {
            return Ok(());
        }

        let child_real = entry.path();
        reject_forbidden_real_path(&child_real)?;

        let name = entry.file_name().to_string_lossy().to_string();
        let child_virtual = virtual_root.join(&name)
            .map_err(|e| Error::Sandbox(format!("virtual path join failed: {e}")))?;

        let metadata = fs::metadata(&child_real)
            .map_err(|e| Error::Sandbox(format!("failed to stat '{}': {e}", child_real.display())))?;
        if metadata.is_dir() {
            if recursive {
                collect_search_matches(&child_real, &child_virtual, query, recursive, limit, matches)?;
            }
            continue;
        }

        let bytes = fs::read(&child_real)
            .map_err(|e| Error::Sandbox(format!("failed to read '{}': {e}", child_real.display())))?;
        let text = String::from_utf8_lossy(&bytes);
        for (line_index, line) in text.lines().enumerate() {
            if matches.len() >= limit {
                return Ok(());
            }
            if let Some(column) = line.find(query) {
                matches.push(SearchMatch {
                    path: child_virtual.clone(),
                    line: Some((line_index + 1) as u64),
                    column: Some((column + 1) as u64),
                    excerpt: line.to_string(),
                });
            }
        }
    }
    Ok(())
}

#[derive(serde::Deserialize)]
struct TextPatchSpec {
    start_line: usize,
    delete_count: usize,
    new_lines: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::LocalSandboxedFilesystem;
    use hstack_core::filesystem::{
        DeleteMode, FilesystemInstruction, FilesystemOutcome, FilesystemPolicy, SearchScope,
        WriteMode,
    };
    use hstack_core::virtual_fs::VirtualPath;
    use std::fs;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("hstack-agent-fs-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap_or_else(|e| panic!("failed to create temp root: {e}"));
        root
    }

    #[test]
    fn local_backend_round_trips_write_and_read() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let path = VirtualPath::from_absolute("/src/main.rs")
            .unwrap_or_else(|e| panic!("virtual path parse failed: {e}"));
        fs_backend
            .write_file(&path, b"fn main() {}\n", WriteMode::CreateOnly, None)
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        let read = fs_backend
            .read_file(&path, 0, 1024)
            .unwrap_or_else(|e| panic!("read failed: {e}"));
        assert_eq!(read.content, b"fn main() {}\n");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_text_finds_matches_recursively() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let path = VirtualPath::from_absolute("/docs/notes.txt")
            .unwrap_or_else(|e| panic!("virtual path parse failed: {e}"));
        fs_backend
            .write_file(&path, b"important\nsecondary\n", WriteMode::CreateOnly, None)
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        let matches = fs_backend
            .search_text(
                &SearchScope {
                    root: VirtualPath::root(),
                    recursive: true,
                },
                "important",
                10,
            )
            .unwrap_or_else(|e| panic!("search failed: {e}"));

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].path.as_str(), "/docs/notes.txt");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn search_text_results_are_deterministic() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        fs_backend
            .write_file(
                &VirtualPath::from_absolute("/zeta.txt")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                b"needle\n",
                WriteMode::CreateOnly,
                None,
            )
            .unwrap_or_else(|e| panic!("write failed: {e}"));
        fs_backend
            .write_file(
                &VirtualPath::from_absolute("/alpha.txt")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                b"needle\n",
                WriteMode::CreateOnly,
                None,
            )
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        let matches = fs_backend
            .search_text(
                &SearchScope {
                    root: VirtualPath::root(),
                    recursive: true,
                },
                "needle",
                10,
            )
            .unwrap_or_else(|e| panic!("search failed: {e}"));

        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].path.as_str(), "/alpha.txt");
        assert_eq!(matches[1].path.as_str(), "/zeta.txt");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_recursive_removes_tree() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let path = VirtualPath::from_absolute("/tmp/a/b.txt")
            .unwrap_or_else(|e| panic!("virtual path parse failed: {e}"));
        fs_backend
            .write_file(&path, b"payload", WriteMode::CreateOnly, None)
            .unwrap_or_else(|e| panic!("write failed: {e}"));

        let deleted = fs_backend
            .delete_path(
                &VirtualPath::from_absolute("/tmp")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                DeleteMode::Recursive,
            )
            .unwrap_or_else(|e| panic!("delete failed: {e}"));

        assert!(deleted.deleted_count >= 2);
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_targets() {
        use std::os::unix::fs::symlink;

        let root = temp_root();
        let real_file = root.join("real.txt");
        fs::write(&real_file, b"payload").unwrap_or_else(|e| panic!("seed write failed: {e}"));
        symlink(&real_file, root.join("link.txt"))
            .unwrap_or_else(|e| panic!("symlink creation failed: {e}"));

        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));
        let err = fs_backend
            .read_file(
                &VirtualPath::from_absolute("/link.txt")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                0,
                16,
            )
            .err()
            .unwrap_or_else(|| panic!("expected symlink rejection"));

        assert!(err.to_string().contains("symlinks are forbidden"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn denies_paths_outside_allowed_virtual_roots() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(
                VirtualPath::from_absolute("/project")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
            ),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let err = fs_backend
            .write_file(
                &VirtualPath::from_absolute("/other/file.txt")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                b"payload",
                WriteMode::CreateOnly,
                None,
            )
            .err()
            .unwrap_or_else(|| panic!("expected policy denial"));

        assert!(err.to_string().contains("not permitted for write"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn denies_normative_ambient_activation_path_classes() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let err = fs_backend
            .write_file(
                &VirtualPath::from_absolute("/.config/autostart/payload.desktop")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                b"[Desktop Entry]",
                WriteMode::CreateOnly,
                None,
            )
            .err()
            .unwrap_or_else(|| panic!("expected ambient path denial"));

        assert!(err.to_string().contains("forbidden path class"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn denies_normative_activation_artifact_classes() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let err = fs_backend
            .write_file(
                &VirtualPath::from_absolute("/project/launch.desktop")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                b"[Desktop Entry]",
                WriteMode::CreateOnly,
                None,
            )
            .err()
            .unwrap_or_else(|| panic!("expected artifact denial"));

        assert!(err.to_string().contains("forbidden artifact class"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn allows_ordinary_inert_project_files() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        fs_backend
            .write_file(
                &VirtualPath::from_absolute("/project/Cargo.toml")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                b"[package]\nname = \"demo\"\n",
                WriteMode::CreateOnly,
                None,
            )
            .unwrap_or_else(|e| panic!("expected inert project file write: {e}"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn patch_file_applies_structured_line_patch() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let path = VirtualPath::from_absolute("/project/file.txt")
            .unwrap_or_else(|e| panic!("virtual path parse failed: {e}"));
        let write = fs_backend
            .write_file(&path, b"alpha\nbeta\ngamma\n", WriteMode::CreateOnly, None)
            .unwrap_or_else(|e| panic!("seed write failed: {e}"));

        fs_backend
            .patch_file(
                &path,
                r#"{"start_line":1,"delete_count":1,"new_lines":["beta-2","beta-3"]}"#,
                write.conflict_token.as_ref(),
            )
            .unwrap_or_else(|e| panic!("patch failed: {e}"));

        let read = fs_backend
            .read_file(&path, 0, 1024)
            .unwrap_or_else(|e| panic!("read failed: {e}"));
        assert_eq!(read.content, b"alpha\nbeta-2\nbeta-3\ngamma\n");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn patch_file_rejects_stale_conflict_token() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let path = VirtualPath::from_absolute("/project/file.txt")
            .unwrap_or_else(|e| panic!("virtual path parse failed: {e}"));
        let write = fs_backend
            .write_file(&path, b"alpha\n", WriteMode::CreateOnly, None)
            .unwrap_or_else(|e| panic!("seed write failed: {e}"));
        fs_backend
            .write_file(&path, b"beta\n", WriteMode::Replace, write.conflict_token.as_ref())
            .unwrap_or_else(|e| panic!("second write failed: {e}"));

        let err = fs_backend
            .patch_file(
                &path,
                r#"{"start_line":0,"delete_count":1,"new_lines":["gamma"]}"#,
                write.conflict_token.as_ref(),
            )
            .err()
            .unwrap_or_else(|| panic!("expected stale token rejection"));

        assert!(err.to_string().contains("conflict token mismatch"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn execute_dispatches_write_instruction() {
        let root = temp_root();
        let fs_backend = LocalSandboxedFilesystem::new(
            root.clone(),
            FilesystemPolicy::project_sandbox(VirtualPath::root()),
        )
        .unwrap_or_else(|e| panic!("backend init failed: {e}"));

        let outcome = fs_backend
            .execute(&FilesystemInstruction::WriteFile {
                path: VirtualPath::from_absolute("/dispatch.txt")
                    .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
                content: b"dispatched".to_vec(),
                mode: WriteMode::CreateOnly,
                expected_conflict_token: None,
            })
            .unwrap_or_else(|e| panic!("dispatch write failed: {e}"));

        match outcome {
            FilesystemOutcome::WriteFile(result) => {
                assert_eq!(result.bytes_written, 10);
            }
            other => panic!("unexpected outcome: {other:?}"),
        }

        let _ = fs::remove_dir_all(root);
    }
}