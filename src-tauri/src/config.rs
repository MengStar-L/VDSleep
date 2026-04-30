use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 1328,
            height: 900,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DisplayConfig {
    pub auto_switch: bool,
    pub target_id: String,
    pub auto_mute: bool,
    pub enhanced_mode: bool,
    pub restore_key_scan_code: u32,
    pub restore_key_label: String,
    pub auto_restore_on_resume: bool,
}

impl Default for DisplayConfig {
    fn default() -> Self {
        Self {
            auto_switch: true,
            target_id: String::new(),
            auto_mute: false,
            enhanced_mode: false,
            restore_key_scan_code: 0,
            restore_key_label: String::new(),
            auto_restore_on_resume: true,
        }
    }
}

pub struct ConfigManager {
    config_dir: PathBuf,
    pub window_config: Mutex<WindowConfig>,
    pub display_config: Mutex<DisplayConfig>,
}

impl ConfigManager {
    pub fn new(config_dir: PathBuf) -> Self {
        fs::create_dir_all(&config_dir).ok();

        let window_config = Self::load_json::<WindowConfig>(&config_dir.join("window.json"));
        let display_config = Self::load_json::<DisplayConfig>(&config_dir.join("display.json"));

        Self {
            config_dir,
            window_config: Mutex::new(window_config),
            display_config: Mutex::new(display_config),
        }
    }

    fn load_json<T: Default + serde::de::DeserializeOwned>(path: &Path) -> T {
        if path.exists() {
            match fs::read_to_string(path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(_) => T::default(),
            }
        } else {
            T::default()
        }
    }

    fn save_json<T: Serialize>(path: &Path, data: &T) {
        if let Ok(json) = serde_json::to_string_pretty(data) {
            fs::write(path, json).ok();
        }
    }

    pub fn save_window_config(&self, config: &WindowConfig) {
        Self::save_json(&self.config_dir.join("window.json"), config);
    }

    pub fn save_display_config(&self, config: &DisplayConfig) {
        Self::save_json(&self.config_dir.join("display.json"), config);
    }
}
