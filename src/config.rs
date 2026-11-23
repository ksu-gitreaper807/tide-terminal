use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "snake_case")]
pub enum GpuMode {
    #[default]
    Integrated,
    Discrete,
    Auto,
    Software,
}
impl GpuMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Integrated => "Integrated GPU",
            Self::Discrete => "Discrete GPU",
            Self::Auto => "Automatic (prefer integrated)",
            Self::Software => "Software (CPU)",
        }
    }
}
macro_rules! group {
    ($name:ident { $($field:ident : $ty:ty = $value:expr),* $(,)? }) => {
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct $name { $(pub $field: $ty),* }
        impl Default for $name { fn default() -> Self { Self { $($field: $value),* } } }
    };
}
group!(Shell {
    program: String = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()),
    working_directory: String = String::new(),
    enhanced_suggestions: bool = true
});
group!(Padding {
    horizontal: i8 = 18,
    vertical: i8 = 6
});
group!(Appearance {
    theme: String = "tide-dark".into(),
    font_family: String = String::new(),
    font_file: String = String::new(),
    font_size: f32 = 16.,
    line_spacing: f32 = 1.,
    opacity: f32 = 0.96,
    padding: Padding = Padding::default()
});
group!(Cursor {
    shape: String = "application".into(),
    blink: bool = true,
    animation: bool = true
});
group!(Tabs {
    show: String = "multiple".into(),
    title: String = "last-command".into(),
    max_title_length: usize = 80
});
group!(Graphics {
    adapter: GpuMode = GpuMode::Integrated,
    animations: bool = true,
    reduced_motion: bool = false,
    animation_duration: f32 = 0.18
});
group!(Notifications {
    input_required: bool = true,
    activate_on_click: bool = true,
    command_finished: bool = false,
    minimum_duration: String = "30s".into()
});
group!(Timers {
    default_unit: String = "seconds".into(),
    stop_grace: String = "3s".into()
});
group!(Scrolling {
    history_lines: usize = 10_000,
    wheel_multiplier: f32 = 1.
});
group!(Privacy {
    private: bool = false
});
group!(Window {
    width: f32 = 1120.,
    height: f32 = 740.
});
group!(Keybindings {
    new_tab: String = "Ctrl+Shift+T".into(),
    run_command: String = "Ctrl+Shift+R".into(),
    append: String = "Ctrl+Shift+A".into(),
    history: String = "Ctrl+Shift+H".into(),
    settings: String = "Ctrl+Shift+Comma".into(),
    close_tab: String = "Ctrl+Shift+W".into(),
    close_window: String = "Ctrl+Shift+Q".into(),
    next_tab: String = "Ctrl+Tab".into(),
    zoom_in: String = "Ctrl+Plus".into()
});

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub shell: Shell,
    pub appearance: Appearance,
    pub cursor: Cursor,
    pub tabs: Tabs,
    pub graphics: Graphics,
    pub notifications: Notifications,
    pub timers: Timers,
    pub scrolling: Scrolling,
    pub privacy: Privacy,
    pub window: Window,
    pub keybindings: Keybindings,
    pub default_profile: Option<String>,
    pub profiles: BTreeMap<String, toml::Table>,
    #[serde(skip)]
    pub active_profile: Option<String>,
    #[serde(skip)]
    original: Option<Box<Config>>,
}

pub fn data_dir() -> PathBuf {
    xdg_path("XDG_DATA_HOME", ".local/share").join("tide-terminal")
}
fn xdg_path(key: &str, fallback: &str) -> PathBuf {
    std::env::var_os(key)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
}
pub fn config_path() -> PathBuf {
    std::env::var_os("TIDE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| xdg_path("XDG_CONFIG_HOME", ".config").join("tide/config.toml"))
}
pub fn expand_home(value: &str) -> PathBuf {
    if value == "~" || value.starts_with("~/") {
        PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
            .join(value.strip_prefix("~/").unwrap_or(""))
    } else {
        PathBuf::from(value)
    }
}
fn normalize(mut table: toml::Table) -> Result<toml::Table> {
    // Only the old flat format used a string at `shell` or scalar notifications.
    let legacy = table.get("shell").is_some_and(toml::Value::is_str)
        || table.get("notifications").is_some_and(toml::Value::is_bool)
        || [
            "gpu",
            "font_size",
            "font_file",
            "opacity",
            "cursor_animation",
            "private",
            "enhanced_suggestions",
        ]
        .iter()
        .any(|k| table.contains_key(*k));
    if legacy {
        for (old, section, field) in [
            ("shell", "shell", "program"),
            ("gpu", "graphics", "adapter"),
            ("font_size", "appearance", "font_size"),
            ("font_file", "appearance", "font_file"),
            ("opacity", "appearance", "opacity"),
            ("cursor_animation", "cursor", "animation"),
            ("notifications", "notifications", "input_required"),
            ("private", "privacy", "private"),
            ("enhanced_suggestions", "shell", "enhanced_suggestions"),
        ] {
            if table.get(old).is_some_and(|v| !v.is_table())
                && let Some(value) = table.remove(old)
            {
                let target = table
                    .entry(section)
                    .or_insert_with(|| toml::Value::Table(toml::Table::new()));
                let target = target
                    .as_table_mut()
                    .context("Mixed legacy and grouped settings")?;
                ensure!(
                    !target.contains_key(field),
                    "Duplicate setting {section}.{field}"
                );
                target.insert(field.into(), value);
            }
        }
    }
    Ok(table)
}
fn overlay(base: &mut toml::Table, patch: &toml::Table) {
    for (key, value) in patch {
        if let (Some(toml::Value::Table(current)), toml::Value::Table(next)) =
            (base.get_mut(key), value)
        {
            overlay(current, next);
        } else {
            base.insert(key.clone(), value.clone());
        }
    }
}
impl Config {
    pub fn load() -> Result<Self> {
        Self::load_profile(None)
    }
    pub fn load_profile(profile: Option<&str>) -> Result<Self> {
        let path = config_path();
        let override_profile = std::env::var("TIDE_PROFILE").ok();
        let selected = profile.or(override_profile.as_deref());
        if std::env::var_os("TIDE_CONFIG").is_some() {
            return Self::load_file(&path, selected);
        }
        if !path.try_exists()? {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
            let legacy = [
                home.join(".tide.toml"),
                xdg_path("XDG_CONFIG_HOME", ".config").join("tide-terminal/config.toml"),
            ];
            let mut migrated = Self::default();
            for old in legacy {
                if old.try_exists()? {
                    migrated = Self::load_file(&old, None)?;
                    break;
                }
            }
            // First writer wins: launching two windows must not clobber either file.
            migrated.original = None;
            migrated.write_to(&path, true)?;
        }
        Self::load_file(&path, selected)
    }
    pub fn parse(text: &str, selected: Option<&str>) -> Result<Self> {
        let mut table = normalize(toml::from_str(text)?)?;
        let base: Self = toml::Value::Table(table.clone()).try_into()?;
        base.validate()?;
        let selected = selected.or(base.default_profile.as_deref());
        // Validate every profile, not just the currently selected one.
        for (name, patch) in &base.profiles {
            ensure!(
                !patch.contains_key("profiles") && !patch.contains_key("default_profile"),
                "Profile {name}: nested profiles/default_profile are not supported"
            );
            let mut merged = table.clone();
            overlay(&mut merged, patch);
            let profile: Self = toml::Value::Table(merged)
                .try_into()
                .with_context(|| format!("Profile {name}"))?;
            profile
                .validate()
                .with_context(|| format!("Profile {name}"))?;
        }
        if let Some(name) = selected {
            overlay(
                &mut table,
                base.profiles
                    .get(name)
                    .with_context(|| format!("Unknown profile {name:?}"))?,
            );
        }
        let mut config: Self = toml::Value::Table(table).try_into()?;
        config.validate()?;
        config.active_profile = selected.map(str::to_owned);
        config.original = Some(Box::new(config.clone()));
        Ok(config)
    }
    fn load_file(path: &Path, selected: Option<&str>) -> Result<Self> {
        if !path.try_exists()? {
            ensure!(
                selected.is_none(),
                "Profile requested but {} does not exist",
                path.display()
            );
            return Ok(Self::default());
        }
        Self::parse(
            &std::fs::read_to_string(path)
                .with_context(|| format!("Read settings {}", path.display()))?,
            selected,
        )
        .with_context(|| format!("Settings {}", path.display()))
    }
    pub fn validate(&self) -> Result<()> {
        fn range(name: &str, value: f32, min: f32, max: f32) -> Result<()> {
            ensure!(
                value.is_finite() && (min..=max).contains(&value),
                "{name} must be between {min} and {max}"
            );
            Ok(())
        }
        range("appearance.font_size", self.appearance.font_size, 10., 32.)?;
        range("appearance.opacity", self.appearance.opacity, 0.3, 1.)?;
        range(
            "appearance.line_spacing",
            self.appearance.line_spacing,
            0.8,
            2.,
        )?;
        range(
            "graphics.animation_duration",
            self.graphics.animation_duration,
            0.,
            1.,
        )?;
        range(
            "scrolling.wheel_multiplier",
            self.scrolling.wheel_multiplier,
            0.1,
            10.,
        )?;
        range("window.width", self.window.width, 640., 7680.)?;
        range("window.height", self.window.height, 420., 4320.)?;
        ensure!(
            (0..=64).contains(&self.appearance.padding.horizontal)
                && (0..=64).contains(&self.appearance.padding.vertical),
            "appearance.padding must be between 0 and 64"
        );
        ensure!(
            self.scrolling.history_lines <= 1_000_000,
            "scrolling.history_lines must be at most 1000000"
        );
        ensure!(
            (8..=256).contains(&self.tabs.max_title_length),
            "tabs.max_title_length must be between 8 and 256"
        );
        ensure!(
            ["multiple", "always", "never"].contains(&self.tabs.show.as_str()),
            "tabs.show must be multiple, always or never"
        );
        ensure!(
            ["last-command", "shell"].contains(&self.tabs.title.as_str()),
            "tabs.title must be last-command or shell"
        );
        ensure!(
            ["application", "beam", "block", "underline"].contains(&self.cursor.shape.as_str()),
            "cursor.shape must be application, beam, block or underline"
        );
        ensure!(
            ["tide-dark", "midnight", "light"].contains(&self.appearance.theme.as_str()),
            "appearance.theme must be tide-dark, midnight or light"
        );
        ensure!(
            ["seconds", "minutes", "hours"].contains(&self.timers.default_unit.as_str()),
            "timers.default_unit must be seconds, minutes or hours"
        );
        ensure!(
            self.stop_grace()? <= Duration::from_secs(300),
            "timers.stop_grace must be at most 5m"
        );
        ensure!(
            !self.shell.program.trim().is_empty(),
            "shell.program cannot be empty"
        );
        self.notification_minimum()?;
        let mut used = std::collections::BTreeSet::new();
        for (name, value) in toml::Table::try_from(&self.keybindings)? {
            let value = value.as_str().unwrap();
            if value.is_empty() {
                continue;
            }
            let binding = parse_binding(value).with_context(|| format!("keybindings.{name}"))?;
            ensure!(used.insert(binding), "Duplicate key binding {value}");
        }
        Ok(())
    }
    pub fn notification_minimum(&self) -> Result<Duration> {
        humantime::parse_duration(&self.notifications.minimum_duration)
            .context("notifications.minimum_duration requires a duration such as 30s")
    }
    pub fn stop_grace(&self) -> Result<Duration> {
        humantime::parse_duration(&self.timers.stop_grace)
            .context("timers.stop_grace requires a duration such as 3s")
    }
    pub fn animation_duration(&self) -> f32 {
        if self.graphics.animations && !self.graphics.reduced_motion {
            self.graphics.animation_duration
        } else {
            0.
        }
    }
    pub fn terminal_colors(&self) -> ([u8; 3], [u8; 3]) {
        match self.appearance.theme.as_str() {
            "light" => ([246, 248, 250], [30, 38, 49]),
            "midnight" => ([8, 12, 20], [213, 220, 229]),
            _ => ([20, 22, 26], [213, 220, 229]),
        }
    }
    pub fn save(&self) -> Result<()> {
        self.write_to(&config_path(), false)
    }
    fn write_to(&self, path: &Path, no_clobber: bool) -> Result<()> {
        use std::io::Write;
        self.validate()?;
        let text = if path.exists() {
            std::fs::read_to_string(path)?
        } else {
            String::new()
        };
        let mut document: toml_edit::DocumentMut = if text.is_empty() {
            include_str!("../config.example.toml").parse()?
        } else {
            text.parse()?
        };
        let legacy = normalize(toml::from_str::<toml::Table>(&text)?)?
            != toml::from_str::<toml::Table>(&text)?;
        // Upgrade explicit legacy-format files on save. Default-path migration
        // writes a new file and never touches the legacy source.
        for key in [
            "shell",
            "gpu",
            "font_size",
            "font_file",
            "opacity",
            "cursor_animation",
            "notifications",
            "private",
            "enhanced_suggestions",
        ] {
            if document
                .get(key)
                .is_some_and(|item| item.as_value().is_some())
            {
                document.remove(key);
            }
        }
        let value = toml::Table::try_from(self)?;
        let original = self
            .original
            .as_deref()
            .map(toml::Table::try_from)
            .transpose()?;
        let changed = if let Some(old) = original {
            difference(&value, &old)
        } else {
            value.clone()
        };
        let missing = text.is_empty() || legacy;
        let target = if let Some(name) = &self.active_profile {
            &mut document["profiles"][name]
        } else {
            document.as_item_mut()
        };
        update_document(target, if missing { &value } else { &changed })?;
        Self::parse(&document.to_string(), self.active_profile.as_deref())?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(document.to_string().as_bytes())?;
        file.as_file().sync_all()?;
        if no_clobber {
            if let Err(e) = file.persist_noclobber(path)
                && e.error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(e.error.into());
            }
        } else {
            file.persist(path)
                .with_context(|| format!("Save settings {}", path.display()))?;
        }
        Ok(())
    }
}
fn difference(new: &toml::Table, old: &toml::Table) -> toml::Table {
    new.iter()
        .filter_map(|(k, v)| {
            if old.get(k) == Some(v) {
                return None;
            }
            let value =
                if let (toml::Value::Table(n), Some(toml::Value::Table(o))) = (v, old.get(k)) {
                    toml::Value::Table(difference(n, o))
                } else {
                    v.clone()
                };
            Some((k.clone(), value))
        })
        .collect()
}
fn update_document(target: &mut toml_edit::Item, table: &toml::Table) -> Result<()> {
    if target.is_none() {
        *target = toml_edit::Item::Table(toml_edit::Table::new());
    }
    for (key, value) in table {
        if let toml::Value::Table(table) = value {
            update_document(&mut target[key], table)?;
        } else {
            let literal = match value {
                toml::Value::Float(n) => {
                    toml::Value::Float((n * 1_000_000.).round() / 1_000_000.).to_string()
                }
                _ => value.to_string(),
            };
            let parsed = format!("value = {literal}").parse::<toml_edit::DocumentMut>()?;
            let mut replacement = parsed["value"].clone();
            if let (Some(old), Some(new)) = (
                target.get(key).and_then(toml_edit::Item::as_value),
                replacement.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            target[key] = replacement;
        }
    }
    Ok(())
}
/// Normalized shortcut components; empty strings disable a GUI action.
pub fn parse_binding(value: &str) -> Result<(bool, bool, bool, String)> {
    let mut parts = value.split('+').collect::<Vec<_>>();
    let key = parts.pop().unwrap_or("");
    let (mut ctrl, mut shift, mut alt) = (false, false, false);
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" if !ctrl => ctrl = true,
            "shift" if !shift => shift = true,
            "alt" if !alt => alt = true,
            _ => bail!("Invalid modifier in {value:?}"),
        }
    }
    let key = match key.to_ascii_lowercase().as_str() {
        "comma" => "Comma".into(),
        "plus" => "Plus".into(),
        "minus" => "Minus".into(),
        "tab" => "Tab".into(),
        "enter" => "Enter".into(),
        "escape" => "Escape".into(),
        k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphanumeric() => k.to_ascii_uppercase(),
        k if k.starts_with('f') && k[1..].parse::<u8>().is_ok_and(|n| (1..=24).contains(&n)) => {
            k.to_ascii_uppercase()
        }
        _ => bail!("Invalid key in {value:?}"),
    };
    ensure!(
        ctrl || alt,
        "Shortcut {value:?} must include Ctrl or Alt to preserve terminal typing"
    );
    Ok((ctrl, shift, alt, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_values_survive_grouped_upgrade_and_private_atomic_save() {
        use std::os::unix::fs::PermissionsExt;
        let text = "shell='/bin/zsh'\ngpu='auto'\nfont_size=19.0\nfont_file='/fonts/mine.ttf'\nopacity=0.75\nnotifications=false\nprivate=true\n";
        let mut config = Config::parse(text, None).unwrap();
        assert_eq!(config.shell.program, "/bin/zsh");
        assert_eq!(config.graphics.adapter, GpuMode::Auto);
        assert!(!config.notifications.input_required);
        assert!(config.privacy.private);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        config.appearance.opacity = 0.65;
        config.write_to(&path, false).unwrap();
        let loaded = Config::load_file(&path, None).unwrap();
        assert_eq!(loaded.shell.program, "/bin/zsh");
        assert_eq!(loaded.appearance.font_size, 19.);
        assert_eq!(loaded.appearance.opacity, 0.65);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    #[test]
    fn saves_preserve_comments_unedited_values_and_external_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let text = "# My Tide preferences\n[appearance] # Canvas\nopacity = 0.8 # keep this note\nfont_size = 18.0\n\n[scrolling]\nwheel_multiplier = 2.0 # my mouse\n";
        std::fs::write(&path, text).unwrap();
        let mut config = Config::load_file(&path, None).unwrap();
        std::fs::write(&path, text.replace("font_size = 18.0", "font_size = 21.0")).unwrap();
        config.appearance.opacity = 0.7;
        config.write_to(&path, false).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        for comment in [
            "# My Tide preferences",
            "# Canvas",
            "# keep this note",
            "# my mouse",
        ] {
            assert!(saved.contains(comment), "{saved}");
        }
        let loaded = Config::load_file(&path, None).unwrap();
        assert_eq!(loaded.appearance.font_size, 21.);
        assert_eq!(loaded.appearance.opacity, 0.7);
        config.appearance.opacity = f32::NAN;
        assert!(config.write_to(&path, false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), saved);
    }
    #[test]
    fn profiles_merge_recursively_and_save_only_changed_overrides() {
        let text = "default_profile='coding'\n[appearance]\nfont_size=16.0\nopacity=0.9\n[profiles.coding.appearance]\nfont_size=20.0 # focused\n[profiles.private.privacy]\nprivate=true\n";
        let mut config = Config::parse(text, None).unwrap();
        assert_eq!(config.active_profile.as_deref(), Some("coding"));
        assert_eq!(config.appearance.opacity, 0.9);
        assert_eq!(config.appearance.font_size, 20.);
        assert!(
            Config::parse(text, Some("private"))
                .unwrap()
                .privacy
                .private
        );
        assert!(Config::parse(text, Some("missing")).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, text).unwrap();
        config.appearance.opacity = 0.7;
        config.write_to(&path, false).unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        let raw: toml::Table = toml::from_str(&saved).unwrap();
        assert_eq!(raw["appearance"]["opacity"].as_float(), Some(0.9));
        assert_eq!(
            raw["profiles"]["coding"]["appearance"]["opacity"].as_float(),
            Some(0.7)
        );
        assert!(saved.contains("# focused"));
        assert!(
            !raw["profiles"]["coding"]
                .as_table()
                .unwrap()
                .contains_key("shell")
        );
    }
    #[test]
    fn invalid_values_typos_and_inactive_profiles_are_rejected() {
        for text in [
            "[appearance]\nfont_size=nan",
            "[appearance]\nopacity=1.1",
            "[appearance]\nfont_sze=12",
            "[graphics]\nadapter='typo'",
            "[scrolling]\nhistory_lines=1000001",
            "[appearance.padding]\nhorizontal=-1",
            "[notifications]\nminimum_duration='soon'",
            "[keybindings]\nnew_tab='T'",
            "[keybindings]\nnew_tab='Ctrl+Shift+R'",
            "[profiles.unused.appearance]\nfont_size=100",
            "[profiles.bad]\ndefault_profile='other'",
            "[timers]\nstop_grace='10m'",
        ] {
            assert!(Config::parse(text, None).is_err(), "Accepted {text}");
        }
    }
    #[test]
    fn explicit_missing_files_do_not_read_other_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        assert_eq!(
            Config::load_file(&missing, None).unwrap().graphics.adapter,
            GpuMode::Integrated
        );
        assert!(Config::load_file(&missing, Some("coding")).is_err());
    }
    #[test]
    fn first_writer_wins_and_existing_invalid_files_are_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let first = Config::default();
        first.write_to(&path, true).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let mut second = Config::default();
        second.appearance.font_size = 25.;
        second.write_to(&path, true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::write(&path, "this is invalid").unwrap();
        assert!(Config::load_file(&path, None).is_err());
        assert!(first.write_to(&path, false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "this is invalid");
    }
    #[test]
    fn renderer_modes_and_behavior_settings_round_trip() {
        for adapter in [
            GpuMode::Integrated,
            GpuMode::Discrete,
            GpuMode::Auto,
            GpuMode::Software,
        ] {
            let mut config = Config::default();
            config.graphics.adapter = adapter;
            config.graphics.reduced_motion = true;
            let decoded = Config::parse(&toml::to_string(&config).unwrap(), None).unwrap();
            assert_eq!(decoded.graphics.adapter, adapter);
            assert_eq!(decoded.animation_duration(), 0.);
            assert_eq!(decoded.stop_grace().unwrap(), Duration::from_secs(3));
        }
        let config=Config::parse("[appearance]\ntheme='light'\n[notifications]\ncommand_finished=true\nminimum_duration='500ms'\n",None).unwrap();
        assert_eq!(config.terminal_colors().0, [246, 248, 250]);
        assert_eq!(
            config.notification_minimum().unwrap(),
            Duration::from_millis(500)
        );
    }
    #[test]
    fn bindings_normalize_case_and_reject_ambiguous_or_plain_typing() {
        assert_eq!(
            parse_binding("ctrl+shift+t").unwrap(),
            (true, true, false, "T".into())
        );
        for bad in ["Ctrl+Ctrl+T", "Ctrl++", "Ctrl+Bogus", "Shift+T", "T"] {
            assert!(parse_binding(bad).is_err());
        }
        assert!(Config::parse("[keybindings]\nnew_tab=''", None).is_ok());
    }
    #[test]
    fn shipped_example_is_valid_and_profiles_are_selectable() {
        let text = include_str!("../config.example.toml");
        Config::parse(text, None).unwrap();
        assert_eq!(
            Config::parse(text, Some("presentation"))
                .unwrap()
                .appearance
                .font_size,
            22.
        );
    }
}
