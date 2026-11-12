use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub shell: String,
    pub gpu: GpuMode,
    pub font_size: f32,
    pub font_file: String,
    pub opacity: f32,
    pub cursor_animation: bool,
    pub notifications: bool,
    pub private: bool,
    pub enhanced_suggestions: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            shell: std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into()),
            gpu: GpuMode::default(),
            font_size: 16.,
            font_file: String::new(),
            opacity: 0.96,
            cursor_animation: true,
            notifications: true,
            private: false,
            enhanced_suggestions: true,
        }
    }
}
pub fn data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share")
        })
        .join("tide-terminal")
}
fn legacy_config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("tide-terminal/config.toml")
}
/// Explicit profile override, otherwise the user's editable dotfile.
pub fn config_path() -> PathBuf {
    std::env::var_os("TIDE_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".tide.toml")
        })
}
impl Config {
    pub fn load() -> Result<Self> {
        let legacy = std::env::var_os("TIDE_CONFIG")
            .is_none()
            .then(legacy_config_path);
        Self::load_from(&config_path(), legacy.as_deref())
    }
    fn load_from(primary: &Path, legacy: Option<&Path>) -> Result<Self> {
        let path = if primary.try_exists()? {
            primary
        } else if let Some(old) = legacy {
            if old.try_exists()? {
                old
            } else {
                return Ok(Self::default());
            }
        } else {
            return Ok(Self::default());
        };
        let mut config: Self = toml::from_str(
            &std::fs::read_to_string(path)
                .with_context(|| format!("Read settings {}", path.display()))?,
        )
        .with_context(|| format!("Parse settings {}", path.display()))?;
        config.font_size = config.font_size.clamp(10., 32.);
        config.opacity = config.opacity.clamp(0.3, 1.);
        Ok(config)
    }
    pub fn save(&self) -> Result<()> {
        self.save_to(&config_path())
    }
    fn save_to(&self, path: &Path) -> Result<()> {
        use std::io::Write;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent)?;
        // Atomic replacement preserves a working config if writing fails. The
        // temporary file is owner-only; the legacy file is never modified.
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(path)
            .with_context(|| format!("Save settings {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dotfile_migration_preserves_legacy_and_canonical_takes_precedence() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("config.toml");
        let dot = dir.path().join(".tide.toml");
        let text = "shell = '/bin/zsh'\ngpu = 'auto'\nfont_size = 19.0\nfont_file = '/fonts/custom.ttf'\nopacity = 0.75\n";
        std::fs::write(&old, text).unwrap();
        let settings = Config::load_from(&dot, Some(&old)).unwrap();
        assert_eq!(settings.shell, "/bin/zsh");
        assert_eq!(settings.font_file, "/fonts/custom.ttf");
        settings.save_to(&dot).unwrap();
        assert_eq!(std::fs::read_to_string(&old).unwrap(), text);
        assert_eq!(
            std::fs::metadata(&dot).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&old, "invalid legacy").unwrap();
        let mut canonical = Config::load_from(&dot, Some(&old)).unwrap();
        assert_eq!(canonical.gpu, GpuMode::Auto);
        assert_eq!(canonical.font_size, 19.0);
        canonical.opacity = 0.65;
        canonical.save_to(&dot).unwrap();
        assert_eq!(Config::load_from(&dot, None).unwrap().opacity, 0.65);
        std::fs::write(&dot, "invalid canonical").unwrap();
        assert!(Config::load_from(&dot, Some(&old)).is_err());
        // An explicit missing profile gets defaults, never another profile's data.
        assert_eq!(
            Config::load_from(&dir.path().join("missing"), None)
                .unwrap()
                .gpu,
            GpuMode::Integrated
        );
    }
    #[test]
    fn existing_configs_keep_integrated_default_and_modes_round_trip() {
        assert_eq!(
            toml::from_str::<Config>("opacity = 0.8").unwrap().gpu,
            GpuMode::Integrated
        );
        for gpu in [
            GpuMode::Integrated,
            GpuMode::Discrete,
            GpuMode::Auto,
            GpuMode::Software,
        ] {
            let config = Config {
                gpu,
                ..Config::default()
            };
            assert_eq!(
                toml::from_str::<Config>(&toml::to_string(&config).unwrap())
                    .unwrap()
                    .gpu,
                gpu
            );
        }
        assert!(toml::from_str::<Config>("gpu = 'typo'").is_err());
    }
}
