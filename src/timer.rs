use anyhow::Result;
use clap::Args;
use std::{ffi::OsString, time::Duration};
use tide_terminal::{
    config::Config,
    rules::RuleSpec,
    session::{Launch, Options},
};

/// Timer shorthand, also available as T_: in integrated shells.
#[derive(Args, Debug)]
#[command(
    after_help = "Examples:\n  T_: 'sleep 60' : -s 10\n  T_: 'python3 -u train.py' : -m 20 -rex 'FATAL:.*'\n  T_: 'worker' : -idle-s 30 -text DONE\n  T_: -s 10 -- sleep 60\n\nQuote the entire shell command. The separating colon is optional.\n-rex, -text, -idle-s, -idle-m and -grace are accepted spellings of the long options.\nThe first matching condition interrupts the command, then force-quits after the grace period."
)]
pub struct TimerArgs {
    /// Shell command, including any pipes or redirects (quote the entire string).
    #[arg(value_name = "SHELL_COMMAND", required_unless_present = "command")]
    command_line: Option<String>,
    /// Maximum runtime in seconds (decimals accepted).
    #[arg(short = 's', long, value_parser = seconds, conflicts_with = "minutes")]
    seconds: Option<Duration>,
    /// Maximum runtime in minutes (decimals accepted).
    #[arg(short = 'm', long, value_parser = minutes)]
    minutes: Option<Duration>,
    /// Stop after this many seconds without visible output.
    #[arg(long, value_parser = seconds, conflicts_with = "idle_minutes")]
    idle_seconds: Option<Duration>,
    /// Stop after this many minutes without visible output.
    #[arg(long, value_parser = minutes)]
    idle_minutes: Option<Duration>,
    /// Stop when visible output matches this regex; does not filter output.
    #[arg(
        long = "rex",
        alias = "regex",
        value_name = "PATTERN",
        allow_hyphen_values = true
    )]
    regex: Option<String>,
    /// Stop when visible output contains this literal text (case sensitive).
    #[arg(
        long,
        alias = "stop-on",
        value_name = "TEXT",
        allow_hyphen_values = true
    )]
    text: Option<String>,
    /// Shell for a quoted command; defaults to the calling Tide shell, then Settings.
    #[arg(long, conflicts_with = "command")]
    shell: Option<String>,
    /// Cleanup time before SIGKILL, with a unit suffix such as 3s or 500ms.
    #[arg(long, default_value = "3s", value_parser = super::parse_duration)]
    grace: Duration,
    /// Send SIGINT only; a command that ignores it can keep running.
    #[arg(long)]
    no_force: bool,
    /// Emit JSON events instead of an interactive transcript.
    #[arg(long)]
    pub json: bool,
    /// Enable desktop input-request notifications.
    #[arg(long)]
    notify: bool,
    /// Do not save the run in Tide history (also inherited from a private Tide shell).
    #[arg(long)]
    private: bool,
    /// Execute an argv directly after --, without a shell.
    #[arg(
        last = true,
        required_unless_present = "command_line",
        conflicts_with = "command_line"
    )]
    command: Vec<String>,
}

fn numeric_duration(value: &str, unit: &str) -> Result<Duration, String> {
    // Humantime handles decimal conversion without floating-point overflow or rounding casts.
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err("Enter a positive number; the flag supplies the unit".into());
    }
    super::parse_duration(&format!("{value}{unit}"))
}
fn seconds(value: &str) -> Result<Duration, String> {
    numeric_duration(value, "s")
}
fn minutes(value: &str) -> Result<Duration, String> {
    numeric_duration(value, "m")
}

impl TimerArgs {
    pub fn options(self) -> Result<Options> {
        let mut options = Options::shell("/bin/bash", std::env::current_dir()?);
        options.launch = if let Some(command) = self.command_line {
            anyhow::ensure!(!command.trim().is_empty(), "A command is required");
            let shell = self.shell.or_else(|| std::env::var("TIDE_SHELL").ok());
            let shell = match shell {
                Some(shell) => shell,
                None => Config::load()?.shell,
            };
            Launch::ShellCommand { shell, command }
        } else {
            Launch::Command(self.command)
        };
        options.rules = RuleSpec {
            timeout: self.seconds.or(self.minutes),
            idle: self.idle_seconds.or(self.idle_minutes),
            literal: self.text,
            pattern: self.regex,
            grace: self.grace,
            force: !self.no_force,
        };
        options.private = self.private || std::env::var_os("TIDE_PRIVATE").is_some();
        options.notifications = self.notify;
        options.history = Some(super::history_path());
        Ok(options)
    }
}

/// Translate the requested single-dash words without touching option values or child argv.
pub fn normalize(mut args: Vec<OsString>) -> Vec<OsString> {
    if args.get(1).is_none_or(|arg| arg != "timer") {
        return args;
    }
    if args
        .get(2)
        .is_some_and(|arg| !arg.to_string_lossy().starts_with('-'))
        && args.get(3).is_some_and(|arg| arg == ":")
    {
        args.remove(3);
    }
    let mut index = 2;
    while index < args.len() {
        let arg = args[index].to_str().unwrap_or("");
        if arg == "--" {
            break;
        }
        let canonical = match arg {
            "-rex" => "--rex",
            "-text" => "--text",
            "-idle-s" => "--idle-seconds",
            "-idle-m" => "--idle-minutes",
            "-grace" => "--grace",
            other => other,
        };
        let takes_value = matches!(
            canonical,
            "-s" | "-m"
                | "--seconds"
                | "--minutes"
                | "--idle-seconds"
                | "--idle-minutes"
                | "--rex"
                | "--regex"
                | "--text"
                | "--stop-on"
                | "--shell"
                | "--grace"
        );
        if canonical != arg {
            args[index] = canonical.into();
        }
        index += if takes_value { 2 } else { 1 };
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cli, Commands};
    use clap::Parser;

    fn parse(args: &[&str]) -> Result<TimerArgs, clap::Error> {
        let args = normalize(args.iter().map(OsString::from).collect());
        let cli = Cli::try_parse_from(args)?;
        let Some(Commands::Timer(timer)) = cli.command else {
            panic!("Expected timer")
        };
        Ok(timer)
    }

    #[test]
    fn colon_style_and_units() {
        let a = parse(&[
            "tide",
            "timer",
            "grep -i \"hi\" .",
            ":",
            "-s",
            "1200",
            "-rex",
            "[0-9]",
        ])
        .unwrap();
        assert_eq!(a.seconds, Some(Duration::from_secs(1200)));
        assert_eq!(a.regex.as_deref(), Some("[0-9]"));
        assert_eq!(a.command_line.as_deref(), Some("grep -i \"hi\" ."));
        let a = parse(&[
            "tide", "timer", "sleep 60", "-m", "0.5", "-idle-s", "0.1", "-text", "DONE", "-grace",
            "250ms",
        ])
        .unwrap();
        assert_eq!(a.minutes, Some(Duration::from_secs(30)));
        assert_eq!(a.idle_seconds, Some(Duration::from_millis(100)));
        assert_eq!(a.grace, Duration::from_millis(250));
        assert_eq!(a.text.as_deref(), Some("DONE"));
        let a = parse(&["tide", "timer", "worker", "-idle-m", "2"]).unwrap();
        assert_eq!(a.idle_minutes, Some(Duration::from_secs(120)));
    }

    #[test]
    fn preserve_option_values_and_direct_argv() {
        let a = parse(&["tide", "timer", "worker", "-text", "-rex"]).unwrap();
        assert_eq!(a.text.as_deref(), Some("-rex"));
        let a = parse(&[
            "tide", "timer", "-s", "1", "--", "printf", "%s", "-rex", ":", "a b",
        ])
        .unwrap();
        assert_eq!(a.command, ["printf", "%s", "-rex", ":", "a b"]);
        assert!(a.command_line.is_none());
    }

    #[test]
    fn reject_missing_ambiguous_and_invalid_arguments() {
        for args in [
            vec!["-s", "1"],
            vec!["sleep 60", "-s", "0"],
            vec!["sleep 60", "-m", "-1"],
            vec!["sleep 60", "-s", "1m"],
            vec!["sleep 60", "-s", "1", "-m", "1"],
            vec!["sleep 60", "-idle-s", "1", "-idle-m", "1"],
            vec!["sleep 60", "--", "sleep", "1"],
            vec!["--shell", "/bin/bash", "--", "sleep", "1"],
            vec!["sleep 60", "-rex"],
            vec!["sleep 60", "-s", "NaN"],
        ] {
            let mut full = vec!["tide", "timer"];
            full.extend(args);
            assert!(parse(&full).is_err(), "{full:?}");
        }
    }
}
