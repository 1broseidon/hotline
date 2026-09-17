mod shell;
mod workspace;

use rig::tool::ToolExecutionError;
use std::{
    error::Error,
    fmt::{self, Display},
};

pub(crate) use shell::{CommandOutcome, CommandState, RunCommandArgs};
pub use shell::{RunCommand, shell_available};
pub use workspace::{
    EditFile, FindFiles, ListDirectory, ReadFile, SearchFiles, Workspace, WriteFile,
};

/// Names of the workspace tools Hotline Agent offers, in the order they are
/// registered on the driver. `shell` is absent from the ledger when it
/// cannot be confined.
pub const BUILTIN: &[&str] = &["ls", "read", "grep", "glob", "write", "edit", "shell"];

#[derive(Debug)]
pub struct ToolError {
    kind: ToolErrorKind,
    message: String,
}

#[derive(Debug)]
enum ToolErrorKind {
    InvalidArguments,
    PermissionDenied,
    Other,
}

impl ToolError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            kind: ToolErrorKind::InvalidArguments,
            message: message.into(),
        }
    }

    pub fn permission_denied(message: impl Into<String>) -> Self {
        Self {
            kind: ToolErrorKind::PermissionDenied,
            message: message.into(),
        }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self {
            kind: ToolErrorKind::Other,
            message: message.into(),
        }
    }

    pub fn into_execution_error(self) -> ToolExecutionError {
        match self.kind {
            ToolErrorKind::InvalidArguments => ToolExecutionError::invalid_args(self.message),
            ToolErrorKind::PermissionDenied => ToolExecutionError::refused(self.message),
            ToolErrorKind::Other => ToolExecutionError::other(self.message),
        }
    }
}

impl Display for ToolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ToolError {}

impl From<std::io::Error> for ToolError {
    fn from(error: std::io::Error) -> Self {
        Self {
            kind: ToolErrorKind::Other,
            message: error.to_string(),
        }
    }
}
