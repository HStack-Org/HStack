use crate::virtual_fs::VirtualPath;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FsObjectKind {
    RegularFile,
    Directory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityProfile {
    SafeDataSandbox,
    ProjectSandbox,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WriteMode {
    CreateOnly,
    Truncate,
    Replace,
    ReplaceIfTokenMatches,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeleteMode {
    SinglePath,
    Recursive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListingOrder {
    Lexicographic,
    BackendDefined,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictToken(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectoryEntry {
    pub path: VirtualPath,
    pub name: String,
    pub kind: FsObjectKind,
    pub conflict_token: Option<ConflictToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStat {
    pub path: VirtualPath,
    pub kind: FsObjectKind,
    pub size_bytes: Option<u64>,
    pub conflict_token: Option<ConflictToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadResult {
    pub path: VirtualPath,
    pub offset: u64,
    pub limit: u64,
    pub content: Vec<u8>,
    pub conflict_token: Option<ConflictToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteResult {
    pub path: VirtualPath,
    pub bytes_written: u64,
    pub conflict_token: Option<ConflictToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoveResult {
    pub from: VirtualPath,
    pub to: VirtualPath,
    pub overwritten: bool,
    pub conflict_token: Option<ConflictToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteResult {
    pub path: VirtualPath,
    pub deleted_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchScope {
    pub root: VirtualPath,
    pub recursive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchMatch {
    pub path: VirtualPath,
    pub line: Option<u64>,
    pub column: Option<u64>,
    pub excerpt: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemPolicy {
    pub capability_profile: CapabilityProfile,
    pub readable_roots: Vec<VirtualPath>,
    pub writable_roots: Vec<VirtualPath>,
    pub creatable_roots: Vec<VirtualPath>,
    pub deletable_roots: Vec<VirtualPath>,
    pub max_read_bytes: u64,
    pub max_write_bytes: u64,
    pub max_directory_entries: u64,
    pub max_search_matches: u64,
    pub allowed_object_kinds: Vec<FsObjectKind>,
    pub forbidden_path_classes: Vec<String>,
    pub forbidden_artifact_classes: Vec<String>,
    pub listing_order: ListingOrder,
}

impl FilesystemPolicy {
    pub fn safe_data_sandbox(root: VirtualPath) -> Self {
        Self {
            capability_profile: CapabilityProfile::SafeDataSandbox,
            readable_roots: vec![root.clone()],
            writable_roots: vec![root.clone()],
            creatable_roots: vec![root.clone()],
            deletable_roots: vec![root],
            max_read_bytes: 256 * 1024,
            max_write_bytes: 256 * 1024,
            max_directory_entries: 512,
            max_search_matches: 200,
            allowed_object_kinds: vec![FsObjectKind::RegularFile, FsObjectKind::Directory],
            forbidden_path_classes: default_forbidden_path_classes(),
            forbidden_artifact_classes: default_forbidden_artifact_classes(),
            listing_order: ListingOrder::Lexicographic,
        }
    }

    pub fn project_sandbox(root: VirtualPath) -> Self {
        let mut policy = Self::safe_data_sandbox(root);
        policy.capability_profile = CapabilityProfile::ProjectSandbox;
        policy.max_read_bytes = 1024 * 1024;
        policy.max_write_bytes = 1024 * 1024;
        policy.max_directory_entries = 2_048;
        policy.max_search_matches = 1_000;
        policy
    }
}

fn default_forbidden_path_classes() -> Vec<String> {
    [
        "/Library/LaunchAgents",
        "/Library/LaunchDaemons",
        "/System/Library/LaunchAgents",
        "/System/Library/LaunchDaemons",
        "/Library/StartupItems",
        "/Library/LoginItems",
        "/.config/autostart",
        "/.config/systemd/user",
        "/.local/share/systemd/user",
        "/etc/systemd/system",
        "/.config/upstart",
        "/.kde/Autostart",
        "/.config/plasma-workspace/env",
        "/.config/plasma-workspace/shutdown",
        "/etc/xdg/autostart",
        "/AppData/Roaming/Microsoft/Windows/Start Menu/Programs/Startup",
        "/ProgramData/Microsoft/Windows/Start Menu/Programs/StartUp",
        "/Windows/Start Menu/Programs/Startup",
        "/AppData/Roaming/Microsoft/Windows/SendTo",
        "/var/spool/cron",
        "/var/at/spool",
        "/etc/cron.d",
        "/etc/cron.daily",
        "/etc/cron.hourly",
        "/etc/cron.monthly",
        "/etc/cron.weekly",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn default_forbidden_artifact_classes() -> Vec<String> {
    [
        "*.desktop",
        "*.lnk",
        "*.url",
        "*.service",
        "*.timer",
        "*.socket",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilesystemInstruction {
    ListDir {
        path: VirtualPath,
        limit: Option<u64>,
    },
    Stat {
        path: VirtualPath,
    },
    ReadFile {
        path: VirtualPath,
        offset: u64,
        limit: u64,
    },
    WriteFile {
        path: VirtualPath,
        content: Vec<u8>,
        mode: WriteMode,
        expected_conflict_token: Option<ConflictToken>,
    },
    PatchFile {
        path: VirtualPath,
        patch: String,
        expected_conflict_token: Option<ConflictToken>,
    },
    CreateDir {
        path: VirtualPath,
        recursive: bool,
    },
    MovePath {
        from: VirtualPath,
        to: VirtualPath,
        overwrite: bool,
    },
    DeletePath {
        path: VirtualPath,
        mode: DeleteMode,
    },
    SearchText {
        scope: SearchScope,
        query: String,
        limit: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilesystemOutcome {
    ListDir(Vec<DirectoryEntry>),
    Stat(FileStat),
    ReadFile(ReadResult),
    WriteFile(WriteResult),
    PatchFile(WriteResult),
    CreateDir,
    MovePath(MoveResult),
    DeletePath(DeleteResult),
    SearchText(Vec<SearchMatch>),
}

#[cfg(test)]
mod tests {
    use super::{
        CapabilityProfile, FilesystemInstruction, FilesystemOutcome, FilesystemPolicy, WriteMode,
    };
    use crate::virtual_fs::VirtualPath;

    #[test]
    fn safe_policy_uses_root_for_all_capability_roots() {
        let root = VirtualPath::root();
        let policy = FilesystemPolicy::safe_data_sandbox(root.clone());

        assert_eq!(policy.capability_profile, CapabilityProfile::SafeDataSandbox);
        assert_eq!(policy.readable_roots, vec![root.clone()]);
        assert_eq!(policy.writable_roots, vec![root.clone()]);
        assert_eq!(policy.creatable_roots, vec![root.clone()]);
        assert_eq!(policy.deletable_roots, vec![root]);
    }

    #[test]
    fn project_policy_has_larger_budgets_than_safe_policy() {
        let root = VirtualPath::root();
        let safe = FilesystemPolicy::safe_data_sandbox(root.clone());
        let project = FilesystemPolicy::project_sandbox(root);

        assert_eq!(project.capability_profile, CapabilityProfile::ProjectSandbox);
        assert!(project.max_read_bytes > safe.max_read_bytes);
        assert!(project.max_write_bytes > safe.max_write_bytes);
        assert!(project.max_directory_entries > safe.max_directory_entries);
        assert!(project.max_search_matches > safe.max_search_matches);
    }

    #[test]
    fn safe_policy_includes_normative_forbidden_path_classes() {
        let policy = FilesystemPolicy::safe_data_sandbox(VirtualPath::root());
        assert!(policy
            .forbidden_path_classes
            .contains(&"/.config/autostart".to_string()));
        assert!(policy
            .forbidden_path_classes
            .contains(&"/Library/LaunchAgents".to_string()));
    }

    #[test]
    fn project_policy_allows_ordinary_inert_project_filenames() {
        let policy = FilesystemPolicy::project_sandbox(VirtualPath::root());
        assert!(!policy
            .forbidden_artifact_classes
            .contains(&"Cargo.toml".to_string()));
        assert!(!policy
            .forbidden_artifact_classes
            .contains(&"package.json".to_string()));
    }

    #[test]
    fn instruction_serializes_with_virtual_paths() {
        let instruction = FilesystemInstruction::WriteFile {
            path: VirtualPath::from_absolute("/src/main.rs")
                .unwrap_or_else(|e| panic!("virtual path parse failed: {e}")),
            content: b"fn main() {}\n".to_vec(),
            mode: WriteMode::CreateOnly,
            expected_conflict_token: None,
        };

        let json = serde_json::to_string(&instruction)
            .unwrap_or_else(|e| panic!("instruction serialization failed: {e}"));
        assert!(json.contains("/src/main.rs"));
        assert!(json.contains("CreateOnly"));
    }

    #[test]
    fn outcome_serializes_with_payload_kind() {
        let outcome = FilesystemOutcome::CreateDir;
        let json = serde_json::to_string(&outcome)
            .unwrap_or_else(|e| panic!("outcome serialization failed: {e}"));
        assert!(json.contains("CreateDir"));
    }
}