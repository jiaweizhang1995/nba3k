use anyhow::{anyhow, Result};
use std::path::PathBuf;

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .map(|d| d.join("nba3k"))
}

pub fn read_lang() -> Option<String> {
    let path = config_dir()?.join("lang");
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn write_lang(value: &str) -> Result<()> {
    let dir = config_dir().ok_or_else(|| anyhow!("no config dir"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("lang"), value)?;
    Ok(())
}

pub fn read_god_mode() -> Option<bool> {
    let path = config_dir()?.join("god_mode");
    let value = std::fs::read_to_string(path).ok()?;
    match value.trim().to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" => Some(true),
        "off" | "false" | "0" | "no" => Some(false),
        _ => None,
    }
}

pub fn write_god_mode(enabled: bool) -> Result<()> {
    let dir = config_dir().ok_or_else(|| anyhow!("no config dir"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("god_mode"), if enabled { "on" } else { "off" })?;
    Ok(())
}
