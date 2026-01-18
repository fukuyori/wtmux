//! Configuration and color scheme management for wtmux.
//!
//! This module provides:
//! - TOML configuration file loading from `~/.wtmux/config.toml`
//! - Built-in color schemes (default, solarized, monokai, nord, etc.)
//! - Runtime theme switching
//!
//! # Configuration File
//!
//! The configuration file is located at `~/.wtmux/config.toml`:
//!
//! ```toml
//! # Default shell (optional)
//! shell = "pwsh.exe"
//!
//! # Color scheme: default, solarized-dark, solarized-light,
//! #               monokai, nord, dracula, gruvbox-dark, tokyo-night
//! color_scheme = "tokyo-night"
//!
//! [tab_bar]
//! visible = true
//!
//! [status_bar]
//! visible = true
//! show_time = true
//!
//! [pane]
//! border_style = "single"
//! ```
//!
//! # Available Color Schemes
//!
//! - `default` - Classic terminal colors
//! - `solarized-dark` / `solarized-light` - Ethan Schoonover's Solarized
//! - `monokai` - Sublime Text inspired
//! - `nord` - Arctic, bluish color palette
//! - `dracula` - Dark theme with vibrant colors
//! - `gruvbox-dark` - Retro groove colors
//! - `tokyo-night` - VS Code Tokyo Night theme

use std::fs;
use std::path::PathBuf;
use serde::{Deserialize, Serialize};

/// Main configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Default shell command
    pub shell: Option<String>,
    /// Default codepage
    pub codepage: Option<u32>,
    /// Color scheme name
    pub color_scheme: String,
    /// Tab bar settings
    pub tab_bar: TabBarConfig,
    /// Status bar settings
    pub status_bar: StatusBarConfig,
    /// Pane border settings
    pub pane: PaneConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: None,
            codepage: None,
            color_scheme: "default".to_string(),
            tab_bar: TabBarConfig::default(),
            status_bar: StatusBarConfig::default(),
            pane: PaneConfig::default(),
        }
    }
}

/// Tab bar configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TabBarConfig {
    pub visible: bool,
}

impl Default for TabBarConfig {
    fn default() -> Self {
        Self { visible: true }
    }
}

/// Status bar configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct StatusBarConfig {
    pub visible: bool,
    pub show_time: bool,
}

impl Default for StatusBarConfig {
    fn default() -> Self {
        Self {
            visible: true,
            show_time: true,
        }
    }
}

/// Pane configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneConfig {
    pub border_style: String, // "single", "double", "rounded", "none"
}

impl Default for PaneConfig {
    fn default() -> Self {
        Self {
            border_style: "single".to_string(),
        }
    }
}

impl Config {
    /// Load configuration from file
    pub fn load() -> Self {
        if let Some(path) = Self::get_config_path() {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Ok(config) = toml::from_str(&content) {
                        return config;
                    }
                }
            }
        }
        Self::default()
    }

    /// Save configuration to file
    #[allow(dead_code)]
    pub fn save(&self) -> Result<(), String> {
        if let Some(path) = Self::get_config_path() {
            let content = toml::to_string_pretty(self)
                .map_err(|e| format!("Failed to serialize config: {}", e))?;
            fs::write(&path, content)
                .map_err(|e| format!("Failed to write config: {}", e))?;
            Ok(())
        } else {
            Err("Could not determine config path".to_string())
        }
    }

    /// Get config file path
    fn get_config_path() -> Option<PathBuf> {
        if let Some(home) = home_dir() {
            let wtmux_dir = home.join(".wtmux");
            if !wtmux_dir.exists() {
                let _ = fs::create_dir_all(&wtmux_dir);
            }
            return Some(wtmux_dir.join("config.toml"));
        }
        None
    }

    /// Get the color scheme
    pub fn get_color_scheme(&self) -> ColorScheme {
        ColorScheme::by_name(&self.color_scheme)
    }
}

/// Color definition (RGB)
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Convert to crossterm Color
    pub fn to_crossterm(&self) -> crossterm::style::Color {
        crossterm::style::Color::Rgb {
            r: self.r,
            g: self.g,
            b: self.b,
        }
    }
}

/// Color scheme definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorScheme {
    pub name: String,
    
    // Tab bar colors
    pub tab_bar_bg: Color,
    pub tab_bar_fg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_inactive_bg: Color,
    pub tab_inactive_fg: Color,
    
    // Status bar colors
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub status_prefix_bg: Color,
    pub status_prefix_fg: Color,
    
    // Pane colors
    pub pane_border: Color,
    pub pane_border_active: Color,
    
    // Selection colors
    pub selection_bg: Color,
    pub selection_fg: Color,
    
    // History selector colors
    pub selector_bg: Color,
    pub selector_fg: Color,
    pub selector_selected_bg: Color,
    pub selector_selected_fg: Color,
    pub selector_border: Color,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self::default_scheme()
    }
}

impl ColorScheme {
    /// Default color scheme
    pub fn default_scheme() -> Self {
        Self {
            name: "default".to_string(),
            
            // Tab bar - dark gray background
            tab_bar_bg: Color::new(40, 40, 40),
            tab_bar_fg: Color::new(180, 180, 180),
            tab_active_bg: Color::new(60, 60, 180),
            tab_active_fg: Color::new(255, 255, 255),
            tab_inactive_bg: Color::new(60, 60, 60),
            tab_inactive_fg: Color::new(150, 150, 150),
            
            // Status bar - blue
            status_bar_bg: Color::new(0, 100, 0),
            status_bar_fg: Color::new(255, 255, 255),
            status_prefix_bg: Color::new(200, 200, 0),
            status_prefix_fg: Color::new(0, 0, 0),
            
            // Pane borders
            pane_border: Color::new(80, 80, 80),
            pane_border_active: Color::new(100, 150, 255),
            
            // Selection
            selection_bg: Color::new(255, 255, 255),
            selection_fg: Color::new(0, 0, 0),
            
            // History selector
            selector_bg: Color::new(0, 0, 139),
            selector_fg: Color::new(255, 255, 255),
            selector_selected_bg: Color::new(255, 255, 255),
            selector_selected_fg: Color::new(0, 0, 0),
            selector_border: Color::new(100, 100, 255),
        }
    }

    /// Solarized Dark scheme
    pub fn solarized_dark() -> Self {
        Self {
            name: "solarized-dark".to_string(),
            
            tab_bar_bg: Color::new(0, 43, 54),
            tab_bar_fg: Color::new(147, 161, 161),
            tab_active_bg: Color::new(38, 139, 210),
            tab_active_fg: Color::new(253, 246, 227),
            tab_inactive_bg: Color::new(7, 54, 66),
            tab_inactive_fg: Color::new(101, 123, 131),
            
            status_bar_bg: Color::new(7, 54, 66),
            status_bar_fg: Color::new(147, 161, 161),
            status_prefix_bg: Color::new(181, 137, 0),
            status_prefix_fg: Color::new(0, 43, 54),
            
            pane_border: Color::new(7, 54, 66),
            pane_border_active: Color::new(38, 139, 210),
            
            selection_bg: Color::new(38, 139, 210),
            selection_fg: Color::new(253, 246, 227),
            
            selector_bg: Color::new(0, 43, 54),
            selector_fg: Color::new(147, 161, 161),
            selector_selected_bg: Color::new(38, 139, 210),
            selector_selected_fg: Color::new(253, 246, 227),
            selector_border: Color::new(38, 139, 210),
        }
    }

    /// Solarized Light scheme
    pub fn solarized_light() -> Self {
        Self {
            name: "solarized-light".to_string(),
            
            tab_bar_bg: Color::new(253, 246, 227),
            tab_bar_fg: Color::new(101, 123, 131),
            tab_active_bg: Color::new(38, 139, 210),
            tab_active_fg: Color::new(253, 246, 227),
            tab_inactive_bg: Color::new(238, 232, 213),
            tab_inactive_fg: Color::new(88, 110, 117),
            
            status_bar_bg: Color::new(238, 232, 213),
            status_bar_fg: Color::new(101, 123, 131),
            status_prefix_bg: Color::new(181, 137, 0),
            status_prefix_fg: Color::new(253, 246, 227),
            
            pane_border: Color::new(238, 232, 213),
            pane_border_active: Color::new(38, 139, 210),
            
            selection_bg: Color::new(38, 139, 210),
            selection_fg: Color::new(253, 246, 227),
            
            selector_bg: Color::new(253, 246, 227),
            selector_fg: Color::new(101, 123, 131),
            selector_selected_bg: Color::new(38, 139, 210),
            selector_selected_fg: Color::new(253, 246, 227),
            selector_border: Color::new(38, 139, 210),
        }
    }

    /// Monokai scheme
    pub fn monokai() -> Self {
        Self {
            name: "monokai".to_string(),
            
            tab_bar_bg: Color::new(39, 40, 34),
            tab_bar_fg: Color::new(248, 248, 242),
            tab_active_bg: Color::new(166, 226, 46),
            tab_active_fg: Color::new(39, 40, 34),
            tab_inactive_bg: Color::new(60, 60, 54),
            tab_inactive_fg: Color::new(150, 150, 140),
            
            status_bar_bg: Color::new(60, 60, 54),
            status_bar_fg: Color::new(248, 248, 242),
            status_prefix_bg: Color::new(249, 38, 114),
            status_prefix_fg: Color::new(248, 248, 242),
            
            pane_border: Color::new(60, 60, 54),
            pane_border_active: Color::new(166, 226, 46),
            
            selection_bg: Color::new(73, 72, 62),
            selection_fg: Color::new(248, 248, 242),
            
            selector_bg: Color::new(39, 40, 34),
            selector_fg: Color::new(248, 248, 242),
            selector_selected_bg: Color::new(166, 226, 46),
            selector_selected_fg: Color::new(39, 40, 34),
            selector_border: Color::new(166, 226, 46),
        }
    }

    /// Nord scheme
    pub fn nord() -> Self {
        Self {
            name: "nord".to_string(),
            
            tab_bar_bg: Color::new(46, 52, 64),
            tab_bar_fg: Color::new(216, 222, 233),
            tab_active_bg: Color::new(136, 192, 208),
            tab_active_fg: Color::new(46, 52, 64),
            tab_inactive_bg: Color::new(59, 66, 82),
            tab_inactive_fg: Color::new(147, 161, 181),
            
            status_bar_bg: Color::new(59, 66, 82),
            status_bar_fg: Color::new(216, 222, 233),
            status_prefix_bg: Color::new(163, 190, 140),
            status_prefix_fg: Color::new(46, 52, 64),
            
            pane_border: Color::new(59, 66, 82),
            pane_border_active: Color::new(136, 192, 208),
            
            selection_bg: Color::new(76, 86, 106),
            selection_fg: Color::new(236, 239, 244),
            
            selector_bg: Color::new(46, 52, 64),
            selector_fg: Color::new(216, 222, 233),
            selector_selected_bg: Color::new(136, 192, 208),
            selector_selected_fg: Color::new(46, 52, 64),
            selector_border: Color::new(136, 192, 208),
        }
    }

    /// Dracula scheme
    pub fn dracula() -> Self {
        Self {
            name: "dracula".to_string(),
            
            tab_bar_bg: Color::new(40, 42, 54),
            tab_bar_fg: Color::new(248, 248, 242),
            tab_active_bg: Color::new(189, 147, 249),
            tab_active_fg: Color::new(40, 42, 54),
            tab_inactive_bg: Color::new(68, 71, 90),
            tab_inactive_fg: Color::new(98, 114, 164),
            
            status_bar_bg: Color::new(68, 71, 90),
            status_bar_fg: Color::new(248, 248, 242),
            status_prefix_bg: Color::new(80, 250, 123),
            status_prefix_fg: Color::new(40, 42, 54),
            
            pane_border: Color::new(68, 71, 90),
            pane_border_active: Color::new(189, 147, 249),
            
            selection_bg: Color::new(68, 71, 90),
            selection_fg: Color::new(248, 248, 242),
            
            selector_bg: Color::new(40, 42, 54),
            selector_fg: Color::new(248, 248, 242),
            selector_selected_bg: Color::new(189, 147, 249),
            selector_selected_fg: Color::new(40, 42, 54),
            selector_border: Color::new(189, 147, 249),
        }
    }

    /// Gruvbox Dark scheme
    pub fn gruvbox_dark() -> Self {
        Self {
            name: "gruvbox-dark".to_string(),
            
            tab_bar_bg: Color::new(40, 40, 40),
            tab_bar_fg: Color::new(235, 219, 178),
            tab_active_bg: Color::new(215, 153, 33),
            tab_active_fg: Color::new(40, 40, 40),
            tab_inactive_bg: Color::new(60, 56, 54),
            tab_inactive_fg: Color::new(168, 153, 132),
            
            status_bar_bg: Color::new(60, 56, 54),
            status_bar_fg: Color::new(235, 219, 178),
            status_prefix_bg: Color::new(152, 151, 26),
            status_prefix_fg: Color::new(40, 40, 40),
            
            pane_border: Color::new(60, 56, 54),
            pane_border_active: Color::new(215, 153, 33),
            
            selection_bg: Color::new(102, 92, 84),
            selection_fg: Color::new(235, 219, 178),
            
            selector_bg: Color::new(40, 40, 40),
            selector_fg: Color::new(235, 219, 178),
            selector_selected_bg: Color::new(215, 153, 33),
            selector_selected_fg: Color::new(40, 40, 40),
            selector_border: Color::new(215, 153, 33),
        }
    }

    /// Tokyo Night scheme
    pub fn tokyo_night() -> Self {
        Self {
            name: "tokyo-night".to_string(),
            
            tab_bar_bg: Color::new(26, 27, 38),
            tab_bar_fg: Color::new(169, 177, 214),
            tab_active_bg: Color::new(122, 162, 247),
            tab_active_fg: Color::new(26, 27, 38),
            tab_inactive_bg: Color::new(36, 40, 59),
            tab_inactive_fg: Color::new(86, 95, 137),
            
            status_bar_bg: Color::new(36, 40, 59),
            status_bar_fg: Color::new(169, 177, 214),
            status_prefix_bg: Color::new(158, 206, 106),
            status_prefix_fg: Color::new(26, 27, 38),
            
            pane_border: Color::new(41, 46, 66),
            pane_border_active: Color::new(122, 162, 247),
            
            selection_bg: Color::new(51, 59, 91),
            selection_fg: Color::new(192, 202, 245),
            
            selector_bg: Color::new(26, 27, 38),
            selector_fg: Color::new(169, 177, 214),
            selector_selected_bg: Color::new(122, 162, 247),
            selector_selected_fg: Color::new(26, 27, 38),
            selector_border: Color::new(122, 162, 247),
        }
    }

    /// Get scheme by name
    pub fn by_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "solarized-dark" | "solarized_dark" => Self::solarized_dark(),
            "solarized-light" | "solarized_light" => Self::solarized_light(),
            "monokai" => Self::monokai(),
            "nord" => Self::nord(),
            "dracula" => Self::dracula(),
            "gruvbox-dark" | "gruvbox_dark" | "gruvbox" => Self::gruvbox_dark(),
            "tokyo-night" | "tokyo_night" | "tokyonight" => Self::tokyo_night(),
            _ => Self::default_scheme(),
        }
    }

    /// List available schemes
    pub fn list() -> Vec<&'static str> {
        vec![
            "default",
            "solarized-dark",
            "solarized-light",
            "monokai",
            "nord",
            "dracula",
            "gruvbox-dark",
            "tokyo-night",
        ]
    }
}

// Get home directory
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}
