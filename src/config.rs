use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub shell: String,
    pub font_size: f32,
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
            font_size: 16.,
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
pub fn config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("tide-terminal/config.toml")
}
impl Config {
    pub fn load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut c: Self = toml::from_str(&std::fs::read_to_string(path)?)?;
        c.font_size = c.font_size.clamp(10., 32.);
        c.opacity = c.opacity.clamp(0.3, 1.);
        Ok(c)
    }
    pub fn save(&self) -> Result<()> {
        let p = config_path();
        std::fs::create_dir_all(p.parent().unwrap())?;
        std::fs::write(p, toml::to_string_pretty(self)?)?;
        Ok(())
    }
}
