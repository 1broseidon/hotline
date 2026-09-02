mod shell;
mod utility;
mod workspace;

use rig::tool::ToolExecutionError;
use std::{
    error::Error,
    fmt::{self, Display},
};

pub use shell::RunCommand;
pub use utility::{Calculator, CurrentTime};
pub use workspace::{
    EditFile, FindFiles, ListDirectory, ReadFile, SearchFiles, Workspace, WriteFile,
};

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
