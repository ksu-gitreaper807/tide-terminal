//! Desktop launchers invoke terminals with flags rather than a subcommand.
use std::ffi::OsString;

pub fn normalize(mut args: Vec<OsString>) -> Vec<OsString> {
    if args.get(1).is_some_and(|arg| {
        let arg = arg.to_string_lossy();
        matches!(
            arg.as_ref(),
            "-e" | "--execute" | "--working-directory" | "--hold"
        ) || arg.starts_with("--working-directory=")
            || arg.starts_with("--execute=")
    }) {
        args.insert(1, "gui".into());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Commands};
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
        Cli::try_parse_from(normalize(args.iter().map(OsString::from).collect()))
    }

    #[test]
    fn desktop_exec_preserves_argv_without_shell_interpretation() {
        for prefix in [vec!["tide", "-e"], vec!["tide", "gui", "--execute"]] {
            let mut args = prefix;
            let command = [
                "printf",
                "%s",
                "two words",
                "",
                "$(touch /tmp/never)",
                "--gpu",
                "-e",
            ];
            args.extend(command);
            let Some(Commands::Gui { execute, .. }) = parse(&args).unwrap().command else {
                panic!("expected a GUI command");
            };
            assert_eq!(execute, command);
        }
    }

    #[test]
    fn directory_and_hold_flags_apply_before_the_child_command() {
        for args in [
            vec![
                "tide",
                "--working-directory",
                "/tmp/two words",
                "--hold",
                "-e",
                "sh",
                "-c",
                "exit 3",
            ],
            vec![
                "tide",
                "--working-directory=/tmp/two words",
                "--hold",
                "-e",
                "sh",
                "-c",
                "exit 3",
            ],
        ] {
            let Some(Commands::Gui {
                working_directory,
                hold,
                execute,
                ..
            }) = parse(&args).unwrap().command
            else {
                panic!("expected a GUI command");
            };
            assert_eq!(
                working_directory.unwrap(),
                std::path::Path::new("/tmp/two words")
            );
            assert!(hold);
            assert_eq!(execute, ["sh", "-c", "exit 3"]);
        }
    }

    #[test]
    fn empty_exec_is_rejected_and_existing_subcommands_remain_intact() {
        assert!(parse(&["tide", "-e"]).is_err());
        for args in [
            vec!["tide", "run", "--", "echo", "-e"],
            vec!["tide", "timer", "echo hello", ":", "-s", "1"],
            vec!["tide", "--help"],
        ] {
            let original: Vec<_> = args.into_iter().map(OsString::from).collect();
            assert_eq!(normalize(original.clone()), original);
        }
    }
}
