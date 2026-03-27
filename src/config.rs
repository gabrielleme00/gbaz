use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const APPLICATION: &str = "gbaz";
const CONFIG_FILENAME: &str = "config.toml";

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct EmulatorConfig {
    /// Path to the GBA BIOS file used when loading ROMs.
    pub bios_path: Option<PathBuf>,
}

impl EmulatorConfig {
    /// Loads config from the platform config directory, returning a default if absent or unreadable.
    pub fn load() -> Self {
        let Some(path) = Self::config_file_path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Saves config to the platform config directory, creating it if necessary.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_file_path()
            .ok_or_else(|| "Could not determine config directory".to_string())?;

        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("Failed to create config directory: {e}"))?;
        }

        let contents = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {e}"))?;

        std::fs::write(&path, contents)
            .map_err(|e| format!("Failed to write config: {e}"))
    }

    fn config_file_path() -> Option<PathBuf> {
        ProjectDirs::from("", "", APPLICATION)
            .map(|dirs| dirs.config_dir().join(CONFIG_FILENAME))
    }
}
