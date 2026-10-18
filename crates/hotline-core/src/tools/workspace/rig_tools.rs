//! The Rig tools over a [`Workspace`]: each one parses its arguments, hands
//! them to the workspace on a blocking thread, and returns what it did in
//! words the model reads. The confinement is the workspace's; nothing here
//! resolves a path.

use super::*;
use rig::tool::{Tool, ToolContext, ToolExecutionError};
use serde_json::json;

pub struct ListDirectory {
    workspace: Workspace,
}

impl ListDirectory {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for ListDirectory {
    const NAME: &'static str = "ls";
    type Error = ToolError;
    type Args = ListDirectoryArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "List files and directories at a path. Directories end with a slash. {}",
            self.workspace.paths_reach()
        )
        .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The directory to list. Defaults to the working directory."
                }
            },
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.list_directory(args))
            .await
            .map_err(|error| ToolError::new(format!("Directory task failed: {error}")))?
    }
}

pub struct ReadFile {
    workspace: Workspace,
}

impl ReadFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for ReadFile {
    const NAME: &'static str = "read";
    type Error = ToolError;
    type Args = ReadFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Read a UTF-8 text file with line numbers; each call answers a slice (default {DEFAULT_READ_LINES} lines) and says how many lines remain, and binary content is refused. {}",
            self.workspace.paths_reach()
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to read."
                },
                "start_line": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "First line to return, starting at 1."
                },
                "max_lines": {
                    "type": "integer",
                    "minimum": 1,
                    "description": format!("Lines to return from start_line. Defaults to {DEFAULT_READ_LINES}.")
                }
            },
            "required": ["path"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.read_file(args))
            .await
            .map_err(|error| ToolError::new(format!("File task failed: {error}")))?
    }
}

pub struct SearchFiles {
    workspace: Workspace,
}

impl SearchFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for SearchFiles {
    const NAME: &'static str = "grep";
    type Error = ToolError;
    type Args = SearchFilesArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Search text files under a directory with a regular expression or literal string. Respects ignore files and returns path, line number, and matching line. {}",
            self.workspace.paths_reach()
        )
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Regular expression to search for, or text when literal is true."
                },
                "path": {
                    "type": "string",
                    "description": "The file or directory to search. Defaults to the working directory."
                },
                "glob": {
                    "type": "string",
                    "description": "Optional glob limiting searched files, for example '**/*.rs'."
                },
                "case_insensitive": { "type": "boolean" },
                "literal": { "type": "boolean" },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_SEARCH_RESULTS
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Search hidden files and directories. Defaults to false."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.search_files(args))
            .await
            .map_err(|error| ToolError::new(format!("Search task failed: {error}")))?
    }
}

pub struct FindFiles {
    workspace: Workspace,
}

impl FindFiles {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for FindFiles {
    const NAME: &'static str = "glob";
    type Error = ToolError;
    type Args = FindFilesArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Find files and directories by glob under a directory. Respects ignore files and does not follow directory symlinks. {}",
            self.workspace.paths_reach()
        )
            .to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob matched against paths under the searched directory, for example '**/*.rs'."
                },
                "path": {
                    "type": "string",
                    "description": "The directory to search. Defaults to the working directory."
                },
                "max_results": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": MAX_FIND_RESULTS
                },
                "include_hidden": {
                    "type": "boolean",
                    "description": "Include hidden files and directories. Defaults to false."
                }
            },
            "required": ["pattern"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || workspace.find_files(args))
            .await
            .map_err(|error| ToolError::new(format!("Find task failed: {error}")))?
    }
}

pub struct WriteFile {
    workspace: Workspace,
}

impl WriteFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for WriteFile {
    const NAME: &'static str = "write";
    type Error = ToolError;
    type Args = WriteFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Create or replace a UTF-8 file of up to {} MiB and say how many bytes and lines were written. {}",
            MAX_WRITE_BYTES / (1024 * 1024),
            self.workspace.paths_reach()
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to write."
                },
                "content": {
                    "type": "string",
                    "description": "Complete new UTF-8 file content."
                },
                "overwrite": {
                    "type": "boolean",
                    "description": "Allow replacing an existing file. Defaults to true."
                }
            },
            "required": ["path", "content"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || {
            let mutation = workspace.prepare_write(args)?;
            workspace.commit_mutation(mutation)
        })
        .await
        .map_err(|error| ToolError::other(format!("Write task failed: {error}")))?
    }
}

pub struct EditFile {
    workspace: Workspace,
}

impl EditFile {
    pub fn new(workspace: Workspace) -> Self {
        Self { workspace }
    }
}

impl Tool for EditFile {
    const NAME: &'static str = "edit";
    type Error = ToolError;
    type Args = EditFileArgs;
    type Output = String;

    fn description(&self) -> String {
        format!(
            "Edit a UTF-8 text file by replacing exact text; a non-unique match is rejected unless replace_all is true, writes are capped at {} MiB, and the result says how many bytes and lines were written. {}",
            MAX_WRITE_BYTES / (1024 * 1024),
            self.workspace.paths_reach()
        )
    }

    fn parameters(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The file to edit."
                },
                "old_text": {
                    "type": "string",
                    "description": "Exact text to replace. Include enough context to make it unique."
                },
                "new_text": {
                    "type": "string",
                    "description": "Replacement text."
                },
                "replace_all": {
                    "type": "boolean",
                    "description": "Replace every exact match. Defaults to false."
                }
            },
            "required": ["path", "old_text", "new_text"],
            "additionalProperties": false
        })
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        error.into_execution_error()
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        let workspace = self.workspace.clone();
        tokio::task::spawn_blocking(move || {
            let mutation = workspace.prepare_edit(args)?;
            workspace.commit_mutation(mutation)
        })
        .await
        .map_err(|error| ToolError::other(format!("Edit task failed: {error}")))?
    }
}
