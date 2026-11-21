//! Test migration and overrides in child processes, without mutating test-runner env.
use std::{
    path::Path,
    process::{Command, Output},
};
fn command(home: &Path, args: &[&str], override_file: Option<&Path>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tide-terminal"));
    cmd.args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("xdg"))
        .env("SHELL", "/bin/bash")
        .env_remove("TIDE_CONFIG")
        .env_remove("TIDE_PROFILE");
    if let Some(path) = override_file {
        cmd.env("TIDE_CONFIG", path);
    }
    cmd.output().unwrap()
}
#[test]
fn default_path_migrates_dotfile_once_and_retains_rollback_source() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let old = home.join(".tide.toml");
    let text = "shell='/bin/zsh'\nfont_size=19.0\nopacity=0.75\nprivate=true\n";
    std::fs::write(&old, text).unwrap();
    let xdg_old = home.join("xdg/tide-terminal/config.toml");
    std::fs::create_dir_all(xdg_old.parent().unwrap()).unwrap();
    std::fs::write(&xdg_old, "invalid fallback").unwrap();
    let output = command(home, &["config", "--check"], None);
    assert!(output.status.success(), "{:?}", output);
    let new = home.join("xdg/tide/config.toml");
    let migrated = std::fs::read_to_string(&new).unwrap();
    let config = tide_terminal::config::Config::parse(&migrated, None).unwrap();
    assert_eq!(config.shell.program, "/bin/zsh");
    assert_eq!(config.appearance.font_size, 19.);
    assert!(config.privacy.private);
    assert_eq!(std::fs::read_to_string(&old).unwrap(), text);
    assert!(migrated.contains("# Safe changes reload"));
    std::fs::write(&old, "invalid old settings").unwrap();
    assert!(command(home, &["config", "--check"], None).status.success());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), migrated);
}
#[test]
fn previous_xdg_location_is_migrated_if_no_dotfile_exists() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("xdg/tide-terminal/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "gpu='software'\nfont_size=17.0").unwrap();
    assert!(
        command(dir.path(), &["config", "--check"], None)
            .status
            .success()
    );
    let new = dir.path().join("xdg/tide/config.toml");
    let config =
        tide_terminal::config::Config::parse(&std::fs::read_to_string(new).unwrap(), None).unwrap();
    assert_eq!(
        config.graphics.adapter,
        tide_terminal::config::GpuMode::Software
    );
    assert_eq!(config.appearance.font_size, 17.);
}
#[test]
fn explicit_config_and_cli_profile_are_isolated_from_user_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".tide.toml"), "invalid").unwrap();
    let explicit = home.join("demo.toml");
    std::fs::write(
        &explicit,
        "[appearance]\nfont_size=17.0\n[profiles.talk.appearance]\nfont_size=24.0\n",
    )
    .unwrap();
    let output = command(
        home,
        &["config", "--show", "--profile", "talk"],
        Some(&explicit),
    );
    assert!(output.status.success(), "{:?}", output);
    let config =
        tide_terminal::config::Config::parse(std::str::from_utf8(&output.stdout).unwrap(), None)
            .unwrap();
    assert_eq!(config.appearance.font_size, 24.);
    assert!(!home.join("xdg/tide/config.toml").exists());
    let bad = command(
        home,
        &["config", "--check", "--profile", "missing"],
        Some(&explicit),
    );
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("Unknown profile"));
}
#[test]
fn invalid_new_config_is_reported_without_falling_back_or_replacing_it() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    std::fs::write(home.join(".tide.toml"), "font_size=15.0").unwrap();
    let path = home.join("xdg/tide/config.toml");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text = "[appearance]\nfont_sze=15.0\n";
    std::fs::write(&path, text).unwrap();
    let output = command(home, &["config", "--check"], None);
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(
        error.contains("font_sze") && error.contains("config.toml"),
        "{error}"
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), text);
}
