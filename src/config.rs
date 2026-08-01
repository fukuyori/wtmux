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

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};

/// Main configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Default shell command
    pub shell: Option<String>,
    /// Default codepage
    pub codepage: Option<u32>,
    /// Prefix key (tmux-style notation, e.g., "C-b", "C-a")
    pub prefix_key: String,
    /// Color scheme name
    pub color_scheme: String,
    /// Inject a prompt hook into supported shells to publish the pane cwd.
    pub cwd_prompt_hook: bool,
    /// Default bindings to remove, as key names (`unbind-key`).
    ///
    /// Being a plain array this must appear before any `[table]` in
    /// `config.toml`, or TOML will read it as a key of the preceding table.
    pub unbind: Vec<String>,
    /// Global keybinding settings
    pub keybindings: KeyBindingsConfig,
    /// Prefix-mode bindings: key name -> command (`bind-key`).
    pub bind: BTreeMap<String, String>,
    /// Prefix-less bindings: key name -> command (`bind-key -n`).
    pub bind_root: BTreeMap<String, String>,
    /// Tab bar settings
    pub tab_bar: TabBarConfig,
    /// Status bar settings
    pub status_bar: StatusBarConfig,
    /// Pane border settings
    pub pane: PaneConfig,
    /// Font settings
    pub font: FontConfig,
    /// Pane activity monitor settings (agent multiplexing support)
    pub activity: ActivityConfig,
    /// Commands to run when a pane's agent state changes
    pub hooks: HooksConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: None,
            codepage: None,
            prefix_key: "C-b".to_string(),
            color_scheme: "default".to_string(),
            cwd_prompt_hook: false,
            unbind: Vec::new(),
            keybindings: KeyBindingsConfig::default(),
            bind: BTreeMap::new(),
            bind_root: BTreeMap::new(),
            tab_bar: TabBarConfig::default(),
            status_bar: StatusBarConfig::default(),
            pane: PaneConfig::default(),
            font: FontConfig::default(),
            activity: ActivityConfig::default(),
            hooks: HooksConfig::default(),
        }
    }
}

/// Agent state transition hooks.
///
/// Each entry is a shell command executed (detached, via `cmd /C` on Windows
/// and `sh -c` elsewhere) when a pane's agent state changes to that state.
/// The command receives the transition context through environment variables:
/// `WTMUX_HOOK_STATE`, `WTMUX_HOOK_PREV_STATE`, `WTMUX_HOOK_PANE`
/// (`<window>.<pane>`), `WTMUX_HOOK_WINDOW` (window name), and
/// `WTMUX_HOOK_TITLE` (pane title). Empty string = hook disabled.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HooksConfig {
    /// Run when a pane starts producing output
    pub on_agent_working: String,
    /// Run when a pane looks like it is waiting on the user
    pub on_agent_blocked: String,
    /// Run when a pane finished a burst of work (or its process exited)
    pub on_agent_done: String,
    /// Run when a pane returns to a plain shell prompt
    pub on_agent_idle: String,
}

impl HooksConfig {
    /// Whether any hook command is configured.
    pub fn any_configured(&self) -> bool {
        !self.on_agent_working.is_empty()
            || !self.on_agent_blocked.is_empty()
            || !self.on_agent_done.is_empty()
            || !self.on_agent_idle.is_empty()
    }
}

/// Pane activity monitor configuration.
///
/// Watches every pane for output and bells so that background agents
/// (e.g. AI coding agents) that finish or wait for input get flagged in the
/// tab bar and pane border.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ActivityConfig {
    /// Enable the activity monitor
    pub enabled: bool,
    /// How long a background pane must stay quiet after producing output
    /// before it is flagged as needing attention (milliseconds)
    pub quiet_threshold_ms: u64,
}

impl Default for ActivityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_threshold_ms: 2000,
        }
    }
}

/// Parsed prefix key representation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefixKey {
    /// The character for the prefix key (e.g., 'b' for Ctrl+B)
    pub char: char,
}

impl PrefixKey {
    /// Parse a tmux-style prefix key string.
    ///
    /// Supported formats:
    /// - `"C-b"` / `"C-a"` etc. (Ctrl + letter)
    ///
    /// Returns `None` if the format is invalid.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        // "C-x" format
        if s.len() == 3 && s.starts_with("C-") {
            let ch = s.chars().nth(2)?;
            if ch.is_ascii_alphabetic() {
                return Some(Self { char: ch.to_ascii_lowercase() });
            }
        }
        None
    }

    /// Display name for the prefix key (e.g., "Ctrl+B")
    pub fn display_name(&self) -> String {
        format!("Ctrl+{}", self.char.to_ascii_uppercase())
    }
}

/// Configurable global keybinding strings loaded from `config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeyBindingsConfig {
    /// Open the command history selector.
    pub history_selector: String,
    /// Scroll the viewport up through scrollback.
    pub scrollback_up: String,
    /// Scroll the viewport down through scrollback.
    pub scrollback_down: String,
    /// Jump to the top of scrollback.
    pub scrollback_top: String,
    /// Jump to the bottom/live view.
    pub scrollback_bottom: String,
    /// Extend or move keyboard selection left.
    pub selection_left: String,
    /// Extend or move keyboard selection right.
    pub selection_right: String,
    /// Extend or move keyboard selection up.
    pub selection_up: String,
    /// Extend or move keyboard selection down.
    pub selection_down: String,
    /// Copy the current selection to the clipboard.
    pub copy_selection: String,
}

impl Default for KeyBindingsConfig {
    fn default() -> Self {
        Self {
            history_selector: "C-r".to_string(),
            scrollback_up: "S-PageUp".to_string(),
            scrollback_down: "S-PageDown".to_string(),
            scrollback_top: "S-Home".to_string(),
            scrollback_bottom: "S-End".to_string(),
            selection_left: "S-Left".to_string(),
            selection_right: "S-Right".to_string(),
            selection_up: "S-Up".to_string(),
            selection_down: "S-Down".to_string(),
            copy_selection: "C-S-c".to_string(),
        }
    }
}

/// Parsed keybinding used at runtime for configurable global shortcuts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    pub code: KeyCode,
    pub modifiers: KeyModifiers,
}

impl KeyBinding {
    /// Parse a keybinding string.
    ///
    /// Supported formats:
    /// - `"C-r"` / `"Ctrl+R"`
    /// - `"S-PageUp"` / `"Shift+PageUp"`
    /// - `"C-S-c"` / `"Ctrl+Shift+C"`
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        let parts: Vec<&str> = s
            .split(['-', '+'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        if parts.is_empty() {
            return None;
        }

        let mut modifiers = KeyModifiers::empty();
        for modifier in &parts[..parts.len().saturating_sub(1)] {
            match modifier.to_ascii_lowercase().as_str() {
                "c" | "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
                "s" | "shift" => modifiers |= KeyModifiers::SHIFT,
                "a" | "alt" | "meta" | "m" => modifiers |= KeyModifiers::ALT,
                _ => return None,
            }
        }

        let key_part = parts.last()?.to_ascii_lowercase();
        let code = match key_part.as_str() {
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "pageup" | "page_up" => KeyCode::PageUp,
            "pagedown" | "page_down" => KeyCode::PageDown,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backspace" => KeyCode::Backspace,
            "delete" | "del" => KeyCode::Delete,
            "space" => KeyCode::Char(' '),
            _ if key_part.chars().count() == 1 => KeyCode::Char(key_part.chars().next()?),
            _ => return None,
        };

        Some(Self { code, modifiers })
    }

    /// A binding that never matches any key. Produced by setting a
    /// `[keybindings]` entry to `"none"` (or `"off"` / `"disabled"`).
    pub fn disabled() -> Self {
        Self {
            code: KeyCode::Null,
            modifiers: KeyModifiers::empty(),
        }
    }

    pub fn is_disabled(&self) -> bool {
        self.code == KeyCode::Null
    }

    pub fn matches(&self, event: &KeyEvent) -> bool {
        if self.is_disabled() {
            return false;
        }
        let supported_modifiers = KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT;
        if event.modifiers & supported_modifiers != self.modifiers {
            return false;
        }

        match (self.code, event.code) {
            (KeyCode::Char(expected), KeyCode::Char(actual)) => expected.eq_ignore_ascii_case(&actual),
            (expected, actual) => expected == actual,
        }
    }

    pub fn display_name(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_string());
        }

        parts.push(match self.code {
            KeyCode::Null => "none".to_string(),
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(ch) if ch.is_ascii_alphabetic() => ch.to_ascii_uppercase().to_string(),
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::Up => "Up".to_string(),
            KeyCode::Down => "Down".to_string(),
            KeyCode::Left => "Left".to_string(),
            KeyCode::Right => "Right".to_string(),
            KeyCode::PageUp => "PageUp".to_string(),
            KeyCode::PageDown => "PageDown".to_string(),
            KeyCode::Home => "Home".to_string(),
            KeyCode::End => "End".to_string(),
            KeyCode::Enter => "Enter".to_string(),
            KeyCode::Esc => "Esc".to_string(),
            KeyCode::Tab => "Tab".to_string(),
            KeyCode::Backspace => "Backspace".to_string(),
            KeyCode::Delete => "Delete".to_string(),
            _ => format!("{:?}", self.code),
        });

        parts.join("+")
    }
}

/// Parsed global keybindings used by the main event loop.
#[derive(Debug, Clone, Copy)]
pub struct ParsedKeyBindings {
    pub history_selector: KeyBinding,
    pub scrollback_up: KeyBinding,
    pub scrollback_down: KeyBinding,
    pub scrollback_top: KeyBinding,
    pub scrollback_bottom: KeyBinding,
    pub selection_left: KeyBinding,
    pub selection_right: KeyBinding,
    pub selection_up: KeyBinding,
    pub selection_down: KeyBinding,
    pub copy_selection: KeyBinding,
}

impl ParsedKeyBindings {
    pub fn from_config(config: &KeyBindingsConfig) -> Self {
        let defaults = KeyBindingsConfig::default();

        Self {
            history_selector: parse_or_default("keybindings.history_selector", &config.history_selector, &defaults.history_selector),
            scrollback_up: parse_or_default("keybindings.scrollback_up", &config.scrollback_up, &defaults.scrollback_up),
            scrollback_down: parse_or_default("keybindings.scrollback_down", &config.scrollback_down, &defaults.scrollback_down),
            scrollback_top: parse_or_default("keybindings.scrollback_top", &config.scrollback_top, &defaults.scrollback_top),
            scrollback_bottom: parse_or_default("keybindings.scrollback_bottom", &config.scrollback_bottom, &defaults.scrollback_bottom),
            selection_left: parse_or_default("keybindings.selection_left", &config.selection_left, &defaults.selection_left),
            selection_right: parse_or_default("keybindings.selection_right", &config.selection_right, &defaults.selection_right),
            selection_up: parse_or_default("keybindings.selection_up", &config.selection_up, &defaults.selection_up),
            selection_down: parse_or_default("keybindings.selection_down", &config.selection_down, &defaults.selection_down),
            copy_selection: parse_or_default("keybindings.copy_selection", &config.copy_selection, &defaults.copy_selection),
        }
    }
}

fn parse_or_default(setting_name: &str, configured: &str, default: &str) -> KeyBinding {
    // Explicit opt-out: `scrollback_up = "none"` disables the shortcut
    // entirely instead of falling back to the default.
    if matches!(
        configured.trim().to_ascii_lowercase().as_str(),
        "none" | "off" | "disabled"
    ) {
        return KeyBinding::disabled();
    }

    if let Some(binding) = KeyBinding::parse(configured) {
        return binding;
    }

    eprintln!(
        "[wtmux] Invalid {} = {:?}; falling back to {:?}",
        setting_name,
        configured,
        default
    );
    KeyBinding::parse(default).expect("default keybinding must be valid")
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

/// Font configuration
///
/// Note: wtmux runs inside an existing terminal emulator (e.g. Windows Terminal).
/// These settings are stored for reference and can be applied to the host terminal
/// at startup via OSC sequences or Windows Terminal profile settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    /// Font family name (e.g. "Cascadia Code", "Consolas", "JetBrains Mono")
    /// Empty string means "use the host terminal's current font"
    pub family: String,
    /// Font size in points (0 = use host terminal default)
    pub size: u16,
    /// Enable bold rendering
    pub bold: bool,
    /// Enable font ligatures (requires a ligature-capable font)
    pub ligatures: bool,
    /// Suppress SGR 1 (Bold) attribute from being sent to the host terminal.
    ///
    /// Set to `true` when the Nerd Font / Powerline glyphs in your font lack a
    /// Bold face.  In that case the OS font-fallback mechanism substitutes a
    /// different (non-Nerd-Font) Bold face for PUA code points, causing
    /// Powerline separators and icons to render incorrectly or with wrong cell
    /// widths.  Setting this to `true` tells wtmux to ignore the Bold flag when
    /// rendering, so all glyphs stay in the same Regular face.
    ///
    /// Default: false (bold is rendered normally)
    pub suppress_bold: bool,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: String::new(), // inherit from host terminal
            size: 0,               // inherit from host terminal
            bold: false,
            ligatures: true,
            suppress_bold: false,
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
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create config directory: {}", e))?;
            }
            fs::write(&path, content)
                .map_err(|e| format!("Failed to write config: {}", e))?;
            Ok(())
        } else {
            Err("Could not determine config path".to_string())
        }
    }

    /// Get config file path
    fn get_config_path() -> Option<PathBuf> {
        let wtmux_dir = get_data_dir()?;
        Some(wtmux_dir.join("config.toml"))
    }

    /// Get the color scheme
    pub fn get_color_scheme(&self) -> ColorScheme {
        ColorScheme::by_name(&self.color_scheme)
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, KeyBinding, ParsedKeyBindings, KeyBindingsConfig};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    #[test]
    fn parses_tmux_style_binding() {
        let binding = KeyBinding::parse("C-r").unwrap();
        assert_eq!(binding.code, KeyCode::Char('r'));
        assert_eq!(binding.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn parses_display_style_binding() {
        let binding = KeyBinding::parse("Ctrl+Shift+C").unwrap();
        assert_eq!(binding.code, KeyCode::Char('c'));
        assert_eq!(binding.modifiers, KeyModifiers::CONTROL | KeyModifiers::SHIFT);
    }

    #[test]
    fn matches_char_keys_case_insensitively() {
        let binding = KeyBinding::parse("Ctrl+Shift+C").unwrap();
        assert!(binding.matches(&key_event(KeyCode::Char('C'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)));
    }

    #[test]
    fn keybinding_none_disables_the_shortcut() {
        let config = KeyBindingsConfig {
            scrollback_up: "none".to_string(),
            copy_selection: "OFF".to_string(),
            ..KeyBindingsConfig::default()
        };
        let parsed = ParsedKeyBindings::from_config(&config);

        assert!(parsed.scrollback_up.is_disabled());
        assert!(!parsed
            .scrollback_up
            .matches(&key_event(KeyCode::PageUp, KeyModifiers::SHIFT)));
        assert!(parsed.copy_selection.is_disabled());
        assert_eq!(parsed.scrollback_up.display_name(), "none");

        // Untouched entries keep their defaults
        assert!(parsed
            .scrollback_down
            .matches(&key_event(KeyCode::PageDown, KeyModifiers::SHIFT)));
    }

    #[test]
    fn displays_keybinding_for_status_labels() {
        assert_eq!(KeyBinding::parse("C-r").unwrap().display_name(), "Ctrl+R");
        assert_eq!(
            KeyBinding::parse("S-PageUp").unwrap().display_name(),
            "Shift+PageUp"
        );
        assert_eq!(
            KeyBinding::parse("Ctrl+Shift+C").unwrap().display_name(),
            "Ctrl+Shift+C"
        );
    }

    #[test]
    fn falls_back_to_defaults_for_invalid_binding() {
        let mut config = KeyBindingsConfig::default();
        config.history_selector = "bad-binding".to_string();
        let parsed = ParsedKeyBindings::from_config(&config);

        assert_eq!(parsed.history_selector.code, KeyCode::Char('r'));
        assert_eq!(parsed.history_selector.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn cwd_prompt_hook_defaults_to_disabled() {
        assert!(!Config::default().cwd_prompt_hook);
    }

    #[test]
    fn cwd_prompt_hook_can_be_enabled_from_toml() {
        let config: Config = toml::from_str("cwd_prompt_hook = true").unwrap();

        assert!(config.cwd_prompt_hook);
    }
}

/// Get wtmux data directory
///
/// On Windows: `%LOCALAPPDATA%\wtmux` (e.g., `C:\Users\username\AppData\Local\wtmux`)
/// On macOS / Linux: `$XDG_CONFIG_HOME/wtmux` or `~/.config/wtmux`
/// Fallback: `~/.wtmux`
pub fn get_data_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let path = PathBuf::from(local_app_data).join("wtmux");
        return Some(path);
    }

    #[cfg(not(windows))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(xdg).join("wtmux"));
        }
        if let Some(home) = home_dir() {
            return Some(home.join(".config").join("wtmux"));
        }
    }

    // Fallback to home directory
    if let Some(home) = home_dir() {
        return Some(home.join(".wtmux"));
    }

    None
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
            pane_border: Color::new(130, 130, 130),
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
            
            pane_border: Color::new(101, 123, 131),
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
            
            pane_border: Color::new(147, 161, 161),
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
            
            pane_border: Color::new(117, 113, 94),
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
            
            pane_border: Color::new(97, 110, 136),
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
            
            pane_border: Color::new(98, 114, 164),
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
            
            pane_border: Color::new(146, 131, 116),
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
            
            pane_border: Color::new(86, 95, 137),
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
