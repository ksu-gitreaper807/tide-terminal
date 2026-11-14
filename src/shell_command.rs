//! Helpers for noninteractive command shells, without interactive capture hooks.
use std::path::Path;

pub(crate) fn with_helpers(shell: &str, command: &str) -> String {
    let helpers = match Path::new(shell).file_name().and_then(|s| s.to_str()) {
        Some("bash" | "zsh") => {
            "T_:() { \"$TIDE_EXE\" timer \"$@\"; }\nA_:() { \"$TIDE_EXE\" append \"$@\"; }"
        }
        Some("fish") => {
            "function T_:; \"$TIDE_EXE\" timer $argv; end\nfunction A_:; \"$TIDE_EXE\" append $argv; end"
        }
        _ => return command.into(),
    };
    format!("{helpers}\n{command}")
}
