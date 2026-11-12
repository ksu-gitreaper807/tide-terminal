//! Prepare follow-up actions from a pinned command recording, never from UI event delivery.
use crate::{
    capture::Recording,
    session::{Launch, Options},
};
use anyhow::{Result, ensure};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AppendMode {
    #[default]
    AfterSuccess,
    Pipe,
    File,
}
impl AppendMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::AfterSuccess => "&&  After success",
            Self::Pipe => "|  Process captured output",
            Self::File => ">>  Append output to file",
        }
    }
}
pub struct FollowUp {
    pub source: Recording,
    pub mode: AppendMode,
    pub target: String,
    options: Options,
}
pub enum Prepared {
    Command(Box<Options>),
    File {
        path: PathBuf,
        output: Vec<u8>,
        truncated: bool,
    },
    Skipped(i32),
}
impl FollowUp {
    pub fn new(
        source: Recording,
        mode: AppendMode,
        target: String,
        options: Options,
    ) -> Result<Self> {
        ensure!(
            !target.trim().is_empty(),
            if mode == AppendMode::File {
                "Enter a file path"
            } else {
                "Enter a command"
            }
        );
        ensure!(
            !target.contains('\0'),
            "Commands and paths cannot contain NUL"
        );
        Ok(Self {
            source,
            mode,
            target,
            options,
        })
    }
    pub fn prepare(&self) -> Result<Option<Prepared>> {
        let source = self.source.lock().unwrap();
        let Some(code) = source.exit_code else {
            return Ok(None);
        };
        ensure!(
            code >= 0,
            "Source capture is unavailable: exit status is unknown or stream boundaries were lost"
        );
        if self.mode == AppendMode::AfterSuccess && code != 0 {
            return Ok(Some(Prepared::Skipped(code)));
        }
        if self.mode == AppendMode::File {
            let path = PathBuf::from(&self.target);
            return Ok(Some(Prepared::File {
                path: if path.is_absolute() {
                    path
                } else {
                    source.cwd.join(path)
                },
                output: source.output.as_bytes().to_vec(),
                truncated: source.truncated,
            }));
        }
        let mut options = self.options.clone();
        options.cwd = source.cwd.clone();
        options.private |= source.private;
        options.launch = Launch::ShellCommand {
            shell: source.shell.clone(),
            command: self.target.clone(),
        };
        if self.mode == AppendMode::Pipe {
            options.stdin = Some(source.output.as_bytes().to_vec());
        }
        Ok(Some(Prepared::Command(Box::new(options))))
    }
}
pub fn append_file(path: &Path, output: &[u8]) -> Result<usize> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(output)?;
    file.flush()?;
    Ok(output.len())
}
