#[cfg(feature = "desktop")]
use anyhow::Context;
use anyhow::Result;
use clap::{Parser, Subcommand};
use std::{io::Write, path::PathBuf, time::Duration};
use tide_terminal::{
    config,
    history::History,
    rules::RuleSpec,
    session::{Event, Launch, Options, Session},
};

mod terminal_cli;
mod timer;

#[cfg(feature = "desktop")]
mod gpu;
#[cfg(feature = "desktop")]
mod theme;
#[cfg(feature = "desktop")]
mod ui;

#[derive(Parser)]
#[command(
    version,
    about = "Tide — a native terminal with input alerts and command rules"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Subcommand)]
enum Commands {
    /// Open the native terminal window.
    Gui {
        /// Start in this directory (also supported without the gui subcommand).
        #[arg(long)]
        working_directory: Option<PathBuf>,
        /// Keep a command's terminal open after it exits.
        #[arg(long)]
        hold: bool,
        /// Execute an argv in the window; all remaining arguments belong to it.
        #[arg(short = 'e', long = "execute", num_args = 1.., trailing_var_arg = true, allow_hyphen_values = true)]
        execute: Vec<String>,
        /// Renderer override for this launch: integrated (default), discrete, auto or software.
        #[arg(long, value_enum)]
        gpu: Option<config::GpuMode>,
        #[arg(long)]
        shell: Option<String>,
        #[arg(long)]
        clean: bool,
        #[arg(long)]
        private: bool,
        #[arg(long)]
        demo: bool,
        #[arg(long, hide = true)]
        qa: bool,
        #[arg(long, hide = true)]
        screenshot: Option<PathBuf>,
        #[arg(long,hide=true,value_parser=parse_duration)]
        exit_after: Option<Duration>,
        #[arg(long, hide = true)]
        event_log: Option<PathBuf>,
    },
    /// Supervise an executable in a real PTY, with timer and output rules.
    Run {
        #[arg(long,value_parser=parse_duration)]
        timeout: Option<Duration>,
        #[arg(long,value_parser=parse_duration)]
        idle: Option<Duration>,
        #[arg(long)]
        stop_on: Option<String>,
        #[arg(long)]
        regex: Option<String>,
        #[arg(long,default_value="3s",value_parser=parse_duration)]
        grace: Duration,
        #[arg(long)]
        no_force: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        notify: bool,
        #[arg(long)]
        private: bool,
        #[arg(required = true, trailing_var_arg = true)]
        command: Vec<String>,
    },
    /// Timer shorthand: T_: 'command' : -s 1200 -rex 'PATTERN'.
    Timer(timer::TimerArgs),
    /// Process the last completed command's captured output: A_: '| grep hello'.
    #[command(
        after_help = "Quote the entire fragment as one argument. Submit each example separately:\n  A_: \"&& whoami | A_: '| grep k'\"\n  A_: \"&& T_: 'whoami' : -s 5\"\nUse A_: and T_: (underscore before colon). Run the source by itself and wait for a new prompt before A_:. Do not paste the source and A_: together as one block."
    )]
    Append {
        #[arg(allow_hyphen_values = true)]
        fragment: String,
    },
    /// Look up command history. Ctrl-X Ctrl-H invokes this in integrated shells.
    History {
        #[arg(long, default_value = "")]
        query: String,
        #[arg(long, default_value = "")]
        cwd: String,
        #[arg(long)]
        first: bool,
        #[arg(long)]
        json: bool,
    },
    /// Get a history-based completion without executing it.
    Suggest {
        #[arg(long)]
        prefix: String,
        #[arg(long, default_value = "")]
        cwd: String,
    },
}
fn parse_duration(s: &str) -> std::result::Result<Duration, String> {
    let d = humantime::parse_duration(s).map_err(|e| e.to_string())?;
    if d.is_zero() {
        Err("Duration must be greater than zero".into())
    } else {
        Ok(d)
    }
}
pub fn history_path() -> PathBuf {
    std::env::var_os("TIDE_HISTORY")
        .map(PathBuf::from)
        .unwrap_or_else(|| config::data_dir().join("history.sqlite3"))
}
fn main() {
    if let Err(error) = run() {
        eprintln!("tide: {error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    match Cli::parse_from(timer::normalize(terminal_cli::normalize(
        std::env::args_os().collect(),
    )))
    .command
    {
        Some(Commands::Run {
            timeout,
            idle,
            stop_on,
            regex,
            grace,
            no_force,
            json,
            notify,
            private,
            command,
        }) => {
            let mut options = Options::shell("/bin/bash", std::env::current_dir()?);
            options.launch = Launch::Command(command);
            options.rules = RuleSpec {
                timeout,
                idle,
                literal: stop_on,
                pattern: regex,
                grace,
                force: !no_force,
            };
            options.private = private;
            options.notifications = notify;
            options.history = Some(history_path());
            supervise(options, json)?;
        }
        Some(Commands::Timer(args)) => {
            let json = args.json;
            supervise(args.options()?, json)?;
        }
        Some(Commands::Append { fragment }) => {
            std::process::exit(tide_terminal::append_cli::run(&fragment)?);
        }
        Some(Commands::History {
            query,
            cwd,
            first,
            json,
        }) => {
            if std::env::var_os("TIDE_PRIVATE").is_some() {
                return Ok(());
            }
            let h = History::open(&history_path())?;
            let rows = h.search(&query, &cwd, if first { 1 } else { 100 })?;
            if json {
                println!("{}", serde_json::to_string(&rows)?);
            } else {
                for row in rows {
                    println!("{}", row.command);
                }
            }
        }
        Some(Commands::Suggest { prefix, cwd }) => {
            if std::env::var_os("TIDE_PRIVATE").is_some() {
                return Ok(());
            }
            if let Some(s) = History::open(&history_path())?.suggest(&prefix, &cwd)? {
                println!("{s}");
            }
        }
        command => {
            #[cfg(feature = "desktop")]
            {
                let mut opts = ui::UiOptions::default();
                if let Some(Commands::Gui {
                    working_directory,
                    hold,
                    execute,
                    gpu,
                    shell,
                    clean,
                    private,
                    demo,
                    qa,
                    screenshot,
                    exit_after,
                    event_log,
                }) = command
                {
                    opts = ui::UiOptions {
                        working_directory,
                        hold,
                        execute,
                        gpu,
                        shell,
                        clean,
                        private,
                        demo,
                        qa,
                        screenshot,
                        exit_after,
                        event_log,
                    };
                }
                ui::launch(opts).context("Could not start native window")?;
            }
            #[cfg(not(feature = "desktop"))]
            {
                let _ = command;
                anyhow::bail!("Built without desktop support; use the run or history command");
            }
        }
    }
    Ok(())
}
fn bail_session() -> Result<()> {
    anyhow::bail!("Terminal session ended without an exit event")
}

fn supervise(mut options: Options, json: bool) -> Result<()> {
    let mut size = tide_terminal::platform::terminal_size();
    if let Some((rows, cols)) = size {
        options.rows = rows;
        options.cols = cols;
    }
    let session = Session::spawn(options)?;
    let raw = tide_terminal::platform::RawTerminal::enter(!json)?;
    // Forward input independently; blocking stdin must never delay timer enforcement.
    let input = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let queue = input.clone();
    std::thread::spawn(move || {
        use std::io::Read;
        let mut stdin = std::io::stdin();
        let mut b = [0; 1024];
        while let Ok(n) = stdin.read(&mut b) {
            if n == 0 {
                // Canonical PTY input uses EOT to finish a finite stdin stream.
                queue.lock().unwrap().push(4);
                break;
            }
            queue.lock().unwrap().extend_from_slice(&b[..n]);
        }
    });
    let code = loop {
        let new_size = tide_terminal::platform::terminal_size();
        if new_size != size {
            if let Some((rows, cols)) = new_size {
                session.resize(rows, cols);
            }
            size = new_size;
        }
        let bytes = std::mem::take(&mut *input.lock().unwrap());
        if !bytes.is_empty() {
            forward_input(&session, bytes)?;
        }
        match session.events.recv_timeout(Duration::from_millis(20)) {
            Ok(event) => {
                if json {
                    println!("{}", serde_json::to_string(&event)?);
                } else {
                    match &event {
                        Event::Output { text } => {
                            print!("{text}");
                            std::io::stdout().flush()?;
                        }
                        Event::Attention { message, .. }
                        | Event::Stopping {
                            reason: message, ..
                        }
                        | Event::Error { message } => eprintln!("[tide] {message}"),
                        _ => {}
                    }
                }
                if let Event::Exited { code, .. } = event {
                    break code;
                }
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => bail_session()?,
            Err(_) => {}
        }
    };
    drop(session);
    drop(raw);
    std::process::exit(code.min(255) as i32);
}

fn forward_input(session: &Session, bytes: Vec<u8>) -> Result<()> {
    match session.write(bytes) {
        // The child can exit between draining stdin and forwarding its last bytes.
        // Continue draining exit/output events rather than replacing its result.
        Err(_) if !session.is_running() => Ok(()),
        result => result,
    }
}

#[cfg(test)]
mod supervisor_tests {
    use super::*;
    #[test]
    fn queued_stdin_eof_after_process_exit_does_not_override_exit_status() {
        let mut options = Options::shell("/bin/bash", std::env::temp_dir());
        options.private = true;
        options.launch = Launch::Command(vec!["/bin/true".into()]);
        let session = Session::spawn(options).unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            assert!(std::time::Instant::now() < deadline);
            if let Ok(Event::Exited { code, .. }) =
                session.events.recv_timeout(Duration::from_millis(20))
            {
                assert_eq!(code, 0);
                break;
            }
        }
        // Wait on the actual closed channel, rather than an assumed sleep interval.
        while session.write(vec![4]).is_ok() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(!session.is_running());
        assert!(forward_input(&session, vec![4]).is_ok());
    }
}
