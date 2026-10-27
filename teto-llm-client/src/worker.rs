use std::{env, ffi::OsString, path::PathBuf};

use crate::ClientError;

/// An explicit worker command.
#[derive(Debug, Clone)]
pub struct WorkerLaunch {
    /// Program to execute.
    pub program: PathBuf,
    /// Program arguments.
    pub args: Vec<OsString>,
}

impl WorkerLaunch {
    /// Create a launch for one program.
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    /// Add program arguments.
    pub fn with_args(mut self, args: impl IntoIterator<Item = OsString>) -> Self {
        self.args.extend(args);
        self
    }

    /// Use `TETO_LLM_WORKER`, or the sibling `teto-llm-worker` binary.
    pub fn from_environment() -> Result<Self, ClientError> {
        if let Some(path) = env::var_os("TETO_LLM_WORKER") {
            return Ok(Self::new(PathBuf::from(path)));
        }
        let program = env::current_exe()
            .map_err(|error| ClientError::Io {
                operation: "current_exe",
                kind: error.kind(),
            })?
            .with_file_name("teto-llm-worker");
        Ok(Self::new(program))
    }
}
