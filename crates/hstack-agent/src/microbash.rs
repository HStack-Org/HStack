use std::fmt;

use hstack_core::filesystem::{
    ConflictToken, DeleteMode, FilesystemInstruction, SearchScope, WriteMode,
};
use hstack_core::virtual_fs::{VirtualPath, VirtualPathError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Ls {
        path: Option<String>,
        limit: Option<u64>,
    },
    Cat {
        path: String,
        offset: u64,
        limit: u64,
    },
    Mkdir {
        path: String,
        recursive: bool,
    },
    Mv {
        from: String,
        to: String,
        overwrite: bool,
    },
    Rm {
        path: String,
        recursive: bool,
    },
    Write {
        path: String,
        content: String,
        mode: WriteMode,
        expected_conflict_token: Option<String>,
    },
    Grep {
        query: String,
        root: String,
        recursive: bool,
        limit: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MicrobashError {
    Parse(String),
    UnsupportedConstruct(String),
    UnsupportedCommand(String),
    InvalidOption(String),
    MissingArgument(String),
    InvalidPath(VirtualPathError),
}

impl fmt::Display for MicrobashError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "parse error: {message}"),
            Self::UnsupportedConstruct(message) => write!(f, "unsupported construct: {message}"),
            Self::UnsupportedCommand(command) => write!(f, "unsupported command: {command}"),
            Self::InvalidOption(message) => write!(f, "invalid option: {message}"),
            Self::MissingArgument(message) => write!(f, "missing argument: {message}"),
            Self::InvalidPath(error) => write!(f, "invalid path: {error}"),
        }
    }
}

impl std::error::Error for MicrobashError {}

impl From<VirtualPathError> for MicrobashError {
    fn from(value: VirtualPathError) -> Self {
        Self::InvalidPath(value)
    }
}

pub fn parse_command_line(input: &str) -> Result<CommandLine, MicrobashError> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err(MicrobashError::Parse("command line is empty".to_string()));
    }

    let command = match tokens[0].as_str() {
        "ls" => parse_ls(&tokens[1..])?,
        "cat" => parse_cat(&tokens[1..])?,
        "mkdir" => parse_mkdir(&tokens[1..])?,
        "mv" => parse_mv(&tokens[1..])?,
        "rm" => parse_rm(&tokens[1..])?,
        "write" => parse_write(&tokens[1..])?,
        "grep" => parse_grep(&tokens[1..])?,
        other => return Err(MicrobashError::UnsupportedCommand(other.to_string())),
    };

    Ok(CommandLine { command })
}

pub fn lower_command_line(
    cwd: &VirtualPath,
    command_line: &CommandLine,
) -> Result<Vec<FilesystemInstruction>, MicrobashError> {
    let instruction = match &command_line.command {
        Command::Ls { path, limit } => FilesystemInstruction::ListDir {
            path: resolve_path_token(cwd, path.as_deref().unwrap_or("."))?,
            limit: *limit,
        },
        Command::Cat {
            path,
            offset,
            limit,
        } => FilesystemInstruction::ReadFile {
            path: resolve_path_token(cwd, path)?,
            offset: *offset,
            limit: *limit,
        },
        Command::Mkdir { path, recursive } => FilesystemInstruction::CreateDir {
            path: resolve_path_token(cwd, path)?,
            recursive: *recursive,
        },
        Command::Mv {
            from,
            to,
            overwrite,
        } => FilesystemInstruction::MovePath {
            from: resolve_path_token(cwd, from)?,
            to: resolve_path_token(cwd, to)?,
            overwrite: *overwrite,
        },
        Command::Rm { path, recursive } => FilesystemInstruction::DeletePath {
            path: resolve_path_token(cwd, path)?,
            mode: if *recursive {
                DeleteMode::Recursive
            } else {
                DeleteMode::SinglePath
            },
        },
        Command::Write {
            path,
            content,
            mode,
            expected_conflict_token,
        } => FilesystemInstruction::WriteFile {
            path: resolve_path_token(cwd, path)?,
            content: content.as_bytes().to_vec(),
            mode: *mode,
            expected_conflict_token: expected_conflict_token
                .as_ref()
                .map(|token| ConflictToken(token.clone())),
        },
        Command::Grep {
            query,
            root,
            recursive,
            limit,
        } => FilesystemInstruction::SearchText {
            scope: SearchScope {
                root: resolve_path_token(cwd, root)?,
                recursive: *recursive,
            },
            query: query.clone(),
            limit: *limit,
        },
    };

    Ok(vec![instruction])
}

pub fn parse_and_lower(
    cwd: &VirtualPath,
    input: &str,
) -> Result<Vec<FilesystemInstruction>, MicrobashError> {
    let command_line = parse_command_line(input)?;
    lower_command_line(cwd, &command_line)
}

fn tokenize(input: &str) -> Result<Vec<String>, MicrobashError> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = input.chars().peekable();
    let mut in_quotes = false;

    while let Some(ch) = chars.next() {
        match ch {
            '|' if !in_quotes => {
                return Err(MicrobashError::UnsupportedConstruct(
                    "pipelines are not supported in the public baseline".to_string(),
                ));
            }
            '"' => in_quotes = !in_quotes,
            '\\' if in_quotes => {
                let escaped = chars.next().ok_or_else(|| {
                    MicrobashError::Parse("unterminated escape sequence".to_string())
                })?;
                current.push(match escaped {
                    'n' => '\n',
                    't' => '\t',
                    '"' => '"',
                    '\\' => '\\',
                    other => other,
                });
            }
            ch if ch.is_whitespace() && !in_quotes => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            other => current.push(other),
        }
    }

    if in_quotes {
        return Err(MicrobashError::Parse("unterminated string literal".to_string()));
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    Ok(tokens)
}

fn parse_ls(args: &[String]) -> Result<Command, MicrobashError> {
    let mut path = None;
    let mut limit = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" => {
                limit = Some(parse_u64_option(args, &mut index, "--limit")?);
            }
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("ls does not support '{value}'")));
            }
            value => {
                if path.is_some() {
                    return Err(MicrobashError::Parse("ls accepts at most one path".to_string()));
                }
                path = Some(value.to_string());
            }
        }
        index += 1;
    }
    Ok(Command::Ls { path, limit })
}

fn parse_cat(args: &[String]) -> Result<Command, MicrobashError> {
    let mut path = None;
    let mut offset = 0;
    let mut limit = 16 * 1024;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--offset" => {
                offset = parse_u64_option(args, &mut index, "--offset")?;
            }
            "--limit" => {
                limit = parse_u64_option(args, &mut index, "--limit")?;
            }
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("cat does not support '{value}'")));
            }
            value => {
                if path.is_some() {
                    return Err(MicrobashError::Parse("cat accepts exactly one path".to_string()));
                }
                path = Some(value.to_string());
            }
        }
        index += 1;
    }

    Ok(Command::Cat {
        path: path.ok_or_else(|| MicrobashError::MissingArgument("cat requires a path".to_string()))?,
        offset,
        limit,
    })
}

fn parse_mkdir(args: &[String]) -> Result<Command, MicrobashError> {
    let mut recursive = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "-p" | "--parents" => recursive = true,
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("mkdir does not support '{value}'")));
            }
            value => {
                if path.is_some() {
                    return Err(MicrobashError::Parse("mkdir accepts exactly one path".to_string()));
                }
                path = Some(value.to_string());
            }
        }
    }

    Ok(Command::Mkdir {
        path: path.ok_or_else(|| MicrobashError::MissingArgument("mkdir requires a path".to_string()))?,
        recursive,
    })
}

fn parse_mv(args: &[String]) -> Result<Command, MicrobashError> {
    let mut overwrite = false;
    let mut positional = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--overwrite" => overwrite = true,
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("mv does not support '{value}'")));
            }
            value => positional.push(value.to_string()),
        }
    }

    if positional.len() != 2 {
        return Err(MicrobashError::MissingArgument(
            "mv requires a source path and destination path".to_string(),
        ));
    }

    Ok(Command::Mv {
        from: positional[0].clone(),
        to: positional[1].clone(),
        overwrite,
    })
}

fn parse_rm(args: &[String]) -> Result<Command, MicrobashError> {
    let mut recursive = false;
    let mut path = None;
    for arg in args {
        match arg.as_str() {
            "-r" | "--recursive" => recursive = true,
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("rm does not support '{value}'")));
            }
            value => {
                if path.is_some() {
                    return Err(MicrobashError::Parse("rm accepts exactly one path".to_string()));
                }
                path = Some(value.to_string());
            }
        }
    }

    Ok(Command::Rm {
        path: path.ok_or_else(|| MicrobashError::MissingArgument("rm requires a path".to_string()))?,
        recursive,
    })
}

fn parse_write(args: &[String]) -> Result<Command, MicrobashError> {
    let mut mode = WriteMode::CreateOnly;
    let mut expected_conflict_token = None;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--mode" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    MicrobashError::MissingArgument("--mode requires a value".to_string())
                })?;
                mode = parse_write_mode(value)?;
            }
            "--expected-token" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    MicrobashError::MissingArgument("--expected-token requires a value".to_string())
                })?;
                expected_conflict_token = Some(value.clone());
            }
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("write does not support '{value}'")));
            }
            value => positional.push(value.to_string()),
        }
        index += 1;
    }

    if positional.len() != 2 {
        return Err(MicrobashError::MissingArgument(
            "write requires a path and content string".to_string(),
        ));
    }

    Ok(Command::Write {
        path: positional[0].clone(),
        content: positional[1].clone(),
        mode,
        expected_conflict_token,
    })
}

fn parse_grep(args: &[String]) -> Result<Command, MicrobashError> {
    let mut recursive = false;
    let mut limit = 50;
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "-r" | "--recursive" => recursive = true,
            "--limit" => {
                limit = parse_u64_option(args, &mut index, "--limit")?;
            }
            value if value.starts_with('-') => {
                return Err(MicrobashError::InvalidOption(format!("grep does not support '{value}'")));
            }
            value => positional.push(value.to_string()),
        }
        index += 1;
    }

    if positional.len() != 2 {
        return Err(MicrobashError::MissingArgument(
            "grep requires a query string and root path".to_string(),
        ));
    }

    Ok(Command::Grep {
        query: positional[0].clone(),
        root: positional[1].clone(),
        recursive,
        limit,
    })
}

fn parse_u64_option(
    args: &[String],
    index: &mut usize,
    option: &str,
) -> Result<u64, MicrobashError> {
    *index += 1;
    let value = args.get(*index).ok_or_else(|| {
        MicrobashError::MissingArgument(format!("{option} requires a numeric value"))
    })?;
    value.parse::<u64>().map_err(|_| {
        MicrobashError::InvalidOption(format!("{option} requires a numeric value, got '{value}'"))
    })
}

fn parse_write_mode(value: &str) -> Result<WriteMode, MicrobashError> {
    match value {
        "create_only" => Ok(WriteMode::CreateOnly),
        "truncate" => Ok(WriteMode::Truncate),
        "replace" => Ok(WriteMode::Replace),
        "replace_if_token_matches" => Ok(WriteMode::ReplaceIfTokenMatches),
        other => Err(MicrobashError::InvalidOption(format!(
            "unsupported write mode '{other}'"
        ))),
    }
}

fn resolve_path_token(cwd: &VirtualPath, token: &str) -> Result<VirtualPath, MicrobashError> {
    if token.starts_with('/') {
        Ok(VirtualPath::from_absolute(token)?)
    } else {
        Ok(VirtualPath::resolve(cwd, token)?)
    }
}

#[cfg(test)]
mod tests {
    use hstack_core::filesystem::{DeleteMode, FilesystemInstruction, WriteMode};
    use hstack_core::virtual_fs::VirtualPath;

    use super::{parse_and_lower, parse_command_line, Command, MicrobashError};

    #[test]
    fn lowers_ls_against_cwd_by_default() {
        let cwd = VirtualPath::from_absolute("/project/src")
            .unwrap_or_else(|e| panic!("cwd parse failed: {e}"));
        let instructions = parse_and_lower(&cwd, "ls")
            .unwrap_or_else(|e| panic!("lowering failed: {e}"));

        assert_eq!(
            instructions,
            vec![FilesystemInstruction::ListDir {
                path: cwd,
                limit: None,
            }]
        );
    }

    #[test]
    fn lowers_relative_cat_path() {
        let cwd = VirtualPath::from_absolute("/project/src")
            .unwrap_or_else(|e| panic!("cwd parse failed: {e}"));
        let instructions = parse_and_lower(&cwd, "cat ../README.md --offset 3 --limit 10")
            .unwrap_or_else(|e| panic!("lowering failed: {e}"));

        assert_eq!(
            instructions,
            vec![FilesystemInstruction::ReadFile {
                path: VirtualPath::from_absolute("/project/README.md")
                    .unwrap_or_else(|e| panic!("path parse failed: {e}")),
                offset: 3,
                limit: 10,
            }]
        );
    }

    #[test]
    fn lowers_write_with_mode_and_token() {
        let cwd = VirtualPath::root();
        let instructions = parse_and_lower(
            &cwd,
            "write /notes.txt \"hello world\" --mode replace_if_token_matches --expected-token abc",
        )
        .unwrap_or_else(|e| panic!("lowering failed: {e}"));

        assert_eq!(
            instructions,
            vec![FilesystemInstruction::WriteFile {
                path: VirtualPath::from_absolute("/notes.txt")
                    .unwrap_or_else(|e| panic!("path parse failed: {e}")),
                content: b"hello world".to_vec(),
                mode: WriteMode::ReplaceIfTokenMatches,
                expected_conflict_token: Some(hstack_core::filesystem::ConflictToken(
                    "abc".to_string()
                )),
            }]
        );
    }

    #[test]
    fn lowers_recursive_rm() {
        let cwd = VirtualPath::root();
        let instructions = parse_and_lower(&cwd, "rm -r /tmp")
            .unwrap_or_else(|e| panic!("lowering failed: {e}"));

        assert_eq!(
            instructions,
            vec![FilesystemInstruction::DeletePath {
                path: VirtualPath::from_absolute("/tmp")
                    .unwrap_or_else(|e| panic!("path parse failed: {e}")),
                mode: DeleteMode::Recursive,
            }]
        );
    }

    #[test]
    fn rejects_pipelines_explicitly() {
        let err = parse_command_line("ls | cat /tmp")
            .err()
            .unwrap_or_else(|| panic!("expected pipeline rejection"));
        assert_eq!(
            err,
            MicrobashError::UnsupportedConstruct(
                "pipelines are not supported in the public baseline".to_string()
            )
        );
    }

    #[test]
    fn parses_string_literals() {
        let command = parse_command_line("write relative.txt \"hello\\nworld\"")
            .unwrap_or_else(|e| panic!("parse failed: {e}"));

        assert_eq!(
            command.command,
            Command::Write {
                path: "relative.txt".to_string(),
                content: "hello\nworld".to_string(),
                mode: WriteMode::CreateOnly,
                expected_conflict_token: None,
            }
        );
    }
}