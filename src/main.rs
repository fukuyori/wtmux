//! wtmux — A tmux-like terminal multiplexer for Windows, macOS and Linux
//!
//! wtmux provides tmux-style window/pane management using ConPTY on Windows
//! and POSIX ptys on macOS / Linux. Features include multiple tabs, split
//! panes, familiar keybindings, and accurate rendering of Nerd Font /
//! Powerline prompts (oh-my-posh, Starship).
//!
//! # Features
//!
//! - **Multiple Tabs**: Create and switch between independent workspaces
//! - **Split Panes**: Divide tabs horizontally or vertically
//! - **tmux Keybindings**: Familiar Ctrl+B prefix shortcuts
//! - **Mouse Support**: Click tabs, select text, right-click context menu
//! - **Copy Mode**: vim-style navigation and text selection
//! - **Color Schemes**: 8 built-in themes with runtime switching
//! - **Command History**: Ctrl+R to search and reuse commands
//! - **Nerd Font / Powerline**: oh-my-posh and Starship prompts render correctly
//! - **Shell Integration**: OSC 133/633 for accurate history with modern prompts
//!
//! # Quick Start
//!
//! ```text
//! wtmux              # Start with default shell (cmd.exe)
//! wtmux -7           # Start with PowerShell 7
//! wtmux -w           # Start with WSL
//! ```
//!
//! # Keybindings (Ctrl+B prefix)
//!
//! | Key | Action |
//! |-----|--------|
//! | c | New tab |
//! | n/p | Next/Previous tab |
//! | w | Select a window from the window list |
//! | " | Split horizontal |
//! | % | Split vertical |
//! | x | Close pane |
//! | z | Toggle zoom |
//! | Arrow keys | Navigate panes |

mod core;
mod ui;
mod wm;
mod history;
mod config;
mod copymode;
mod tmux_compat;
mod command_prompt;
mod keybind;

use std::env;
use std::io::Write;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::cursor::SetCursorStyle;
use crossterm::execute;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use crate::core::session::Session;
use crate::core::term::width::{char_width, str_display_width};
use crate::ui::{
    input, ContextMenuAction, KeyMapper, RenameTarget, Renderer, TreeEntry, UiMode, WmAppState,
};
use crate::wm::{WindowManager, SplitDirection};
use crate::config::{ColorScheme, Config as WtmuxConfig, ParsedKeyBindings, PrefixKey};
use crate::keybind::{BindTable, BoundAction};

#[derive(Debug, Clone, Copy, PartialEq)]
enum AppAction {
    Noop,
    NewTab,
    ClosePane,
    CloseTab,
    SplitHorizontal,
    SplitVertical,
    NextTab,
    PrevTab,
    LastTab,
    FocusDirection {
        direction: SplitDirection,
        forward: bool,
    },
    ResizePaneDirection {
        direction: SplitDirection,
        arrow_up_or_left: bool,
    },
    GotoTab(usize),
    FocusNextPane,
    FocusPrevPane,
    ResetCursorShape,
    ToggleZoom,
    NextLayout,
    SelectLayout(crate::wm::layout::LayoutType),
    ResizePane {
        grow: bool,
    },
    SwapPaneNext,
    SwapPanePrev,
    PasteFromClipboard,
    SendPrefixToPane {
        byte: u8,
    },
    ToggleBroadcast,
    FocusNextAttention,
    TogglePipeLog,
    ScrollUp(usize),
    ScrollDown(usize),
    ScrollTop,
    ScrollBottom,
    ExtendSelection(KeyCode),
    CopySelection,
}

impl From<ContextMenuAction> for AppAction {
    fn from(action: ContextMenuAction) -> Self {
        match action {
            ContextMenuAction::Paste => AppAction::PasteFromClipboard,
            ContextMenuAction::KillPane => AppAction::ClosePane,
            ContextMenuAction::SplitHorizontal => AppAction::SplitHorizontal,
            ContextMenuAction::SplitVertical => AppAction::SplitVertical,
            ContextMenuAction::ToggleZoom => AppAction::ToggleZoom,
            // RenamePane needs UI state (the rename popup), so the event loop
            // handles it before converting to an AppAction.
            ContextMenuAction::RenamePane => AppAction::Noop,
            ContextMenuAction::Cancel => AppAction::Noop,
        }
    }
}

/// Application configuration
struct Config {
    /// Default shell command
    shell: Option<String>,
    /// Force native console (skip Windows Terminal detection)
    native_console: bool,
    /// Console codepage (65001 for UTF-8, 932 for Shift-JIS)
    codepage: Option<u32>,
    /// Enable tmux-like multi-pane mode (default: true)
    multipane: bool,
    /// Shell was explicitly set via command line
    shell_from_cli: bool,
    /// Enable debug logging to file
    debug: bool,
    /// Enable VT byte trace to file (--vt-trace)
    vt_trace: bool,
    /// Inject shell prompt hooks that publish pane cwd changes
    cwd_prompt_hook: bool,
    /// cwd prompt hook was explicitly set via command line
    cwd_prompt_hook_from_cli: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: None,  // Will be set from config.toml or default to cmd.exe
            native_console: false,
            codepage: Some(65001), // UTF-8 by default
            multipane: true, // Multi-pane mode is now default
            shell_from_cli: false,
            debug: false,  // Logging disabled by default
            vt_trace: false,
            cwd_prompt_hook: false,
            cwd_prompt_hook_from_cli: false,
        }
    }
}

/// Version string from Cargo.toml
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_version() {
    eprintln!("wtmux {}", VERSION);
}

/// Print the effective binding table (`wtmux list-keys`), after `config.toml`
/// overrides have been applied, in the same syntax `[bind]` accepts.
fn print_keys(wtmux_config: &WtmuxConfig) {
    let prefix = PrefixKey::parse(&wtmux_config.prefix_key).unwrap_or(PrefixKey { char: 'b' });
    let binds = BindTable::build(
        prefix.char,
        &wtmux_config.bind,
        &wtmux_config.bind_root,
        &wtmux_config.unbind,
    );
    for error in binds.errors() {
        eprintln!("[wtmux] {error}");
    }

    for (scope, key, command) in binds.describe() {
        let section = if scope == "root" { "bind_root" } else { "bind" };
        println!("{section:<9} {key:<12} {command}");
    }
}

fn print_help(wtmux_config: &WtmuxConfig) {
    let keybindings = ParsedKeyBindings::from_config(&wtmux_config.keybindings);
    let history_selector = keybindings.history_selector.display_name();
    let prefix_name = PrefixKey::parse(&wtmux_config.prefix_key)
        .unwrap_or(PrefixKey { char: 'b' })
        .display_name();

    eprintln!("wtmux {} - A tmux-like terminal multiplexer", VERSION);
    eprintln!();
    eprintln!("Usage: wtmux [OPTIONS]");
    eprintln!();
    eprintln!("Mode options:");
    eprintln!("  (default)             Multi-pane mode (tmux-like)");
    eprintln!("  -1, --simple          Simple single-pane mode");
    eprintln!();
    eprintln!("Shell options:");
    if cfg!(windows) {
        eprintln!("  (default)             From config.toml or Command Prompt (cmd.exe)");
        eprintln!("  -c, --cmd             Command Prompt (cmd.exe)");
        eprintln!("  -p, --powershell      Windows PowerShell (powershell.exe)");
        eprintln!("  -7, --pwsh            PowerShell 7 (pwsh.exe)");
        eprintln!("  -w, --wsl             WSL (Windows Subsystem for Linux)");
    } else {
        eprintln!("  (default)             From config.toml or $SHELL (/bin/sh fallback)");
    }
    eprintln!("  -s, --shell <CMD>     Custom shell command");
    eprintln!();
    if cfg!(windows) {
        eprintln!("Encoding options:");
        eprintln!("  (default)             UTF-8 (CP65001)");
        eprintln!("  --sjis                Shift-JIS mode (CP932)");
        eprintln!();
    }
    eprintln!("Other options:");
    if cfg!(windows) {
        eprintln!("  -n, --native          Run in native console window");
    }
    eprintln!("  -d, --debug           Enable debug logging to file");
    eprintln!("  --vt-trace            Trace raw PTY bytes to <data dir>/vt_trace.log");
    eprintln!("  -P, --cwd-prompt-hook <on|off>");
    eprintln!("                         Set shell prompt hook cwd tracking");
    eprintln!("  --no-cwd-prompt-hook  Disable shell prompt hook cwd tracking");
    eprintln!("  -v, --version         Show version");
    eprintln!("  -h, --help            Show this help");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  list-keys (lsk)       List the effective key bindings");
    eprintln!();
    eprintln!(
        "Multi-pane mode keybindings (tmux compatible, {} prefix):",
        prefix_name
    );
    eprintln!("  {:<22} New window (tab)", format!("{}, c", prefix_name));
    eprintln!("  {:<22} Kill window (tab)", format!("{}, &", prefix_name));
    eprintln!("  {:<22} Kill pane", format!("{}, x", prefix_name));
    eprintln!(
        "  {:<22} Split pane horizontally (top/bottom)",
        format!("{}, \"", prefix_name)
    );
    eprintln!(
        "  {:<22} Split pane vertically (left/right)",
        format!("{}, %", prefix_name)
    );
    eprintln!("  {:<22} Next window", format!("{}, n", prefix_name));
    eprintln!("  {:<22} Previous window", format!("{}, p", prefix_name));
    eprintln!("  {:<22} Last window (toggle)", format!("{}, l", prefix_name));
    eprintln!("  {:<22} Select window by number", format!("{}, 0-9", prefix_name));
    eprintln!("  {:<22} Next pane", format!("{}, o", prefix_name));
    eprintln!("  {:<22} Previous pane", format!("{}, ;", prefix_name));
    eprintln!(
        "  {:<22} Move to pane in direction",
        format!("{}, Arrow", prefix_name)
    );
    eprintln!("  {:<22} Toggle pane zoom", format!("{}, z", prefix_name));
    eprintln!(
        "  {:<22} Toggle input broadcast to all panes",
        format!("{}, e", prefix_name)
    );
    eprintln!(
        "  {:<22} Jump to next pane needing attention",
        format!("{}, a", prefix_name)
    );
    eprintln!(
        "  {:<22} Agent dashboard (pane states)",
        format!("{}, g", prefix_name)
    );
    eprintln!();
    eprintln!("Snippet selector (at command prompt, not in vim/apps):");
    eprintln!("  {:<22} Open snippet selector", history_selector);
    eprintln!("  ↑/↓                   Navigate snippets");
    eprintln!("  1-9                   Select by number");
    eprintln!("  Enter                 Insert selected snippet");
    eprintln!("  Esc                   Close selector");
    eprintln!("  (type to search)      Filter snippets");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  wtmux                 Multi-pane mode (default)");
    if cfg!(windows) {
        eprintln!("  wtmux -7 -u           PowerShell 7, UTF-8");
        eprintln!("  wtmux -w              WSL");
    } else {
        eprintln!("  wtmux -s zsh          Zsh");
    }
    eprintln!("  wtmux -1              Simple single-pane mode");
    eprintln!();
    if cfg!(windows) {
        eprintln!("Configuration: %LOCALAPPDATA%\\wtmux\\config.toml");
    } else {
        eprintln!("Configuration: ~/.config/wtmux/config.toml");
    }
    eprintln!();
    eprintln!("Color schemes: default, solarized-dark, solarized-light,");
    eprintln!("               monokai, nord, dracula, gruvbox-dark, tokyo-night");
    eprintln!();
    eprintln!("Exit: Type 'exit' in the shell to close pane/tab");
}

fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = env::args().collect();
    let mut config = Config::default();
    let mut i = 1;

    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => {
                let wtmux_config = WtmuxConfig::load();
                print_help(&wtmux_config);
                std::process::exit(0);
            }
            "-v" | "--version" => {
                print_version();
                std::process::exit(0);
            }
            "list-keys" | "lsk" => {
                print_keys(&WtmuxConfig::load());
                std::process::exit(0);
            }
            // Mode selection
            "-1" | "--simple" => {
                config.multipane = false;
            }
            // Shell selection (Windows shell shortcuts)
            #[cfg(windows)]
            "-c" | "--cmd" => {
                config.shell = Some("cmd.exe".to_string());
                config.shell_from_cli = true;
            }
            #[cfg(windows)]
            "-p" | "--powershell" => {
                config.shell = Some("powershell.exe".to_string());
                config.shell_from_cli = true;
            }
            #[cfg(windows)]
            "-7" | "--pwsh" => {
                config.shell = Some("pwsh.exe".to_string());
                config.shell_from_cli = true;
            }
            #[cfg(windows)]
            "-w" | "--wsl" => {
                config.shell = Some("wsl.exe".to_string());
                config.shell_from_cli = true;
                // WSL uses UTF-8 (already default, but explicit)
                config.codepage = Some(65001);
            }
            "-s" | "--shell" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing shell argument".to_string());
                }
                config.shell = Some(args[i].clone());
                config.shell_from_cli = true;
            }
            // Encoding
            "-u" | "--utf8" => {
                config.codepage = Some(65001);
            }
            "--sjis" => {
                config.codepage = Some(932);
            }
            // Other
            "-n" | "--native" => {
                // Will be handled by relaunch logic
            }
            "--no-relaunch" => {
                config.native_console = true;
            }
            "-d" | "--debug" => {
                config.debug = true;
            }
            "--vt-trace" => {
                config.vt_trace = true;
            }
            "-P" => {
                i += 1;
                if i >= args.len() {
                    return Err("Missing cwd prompt hook argument: expected on or off".to_string());
                }
                config.cwd_prompt_hook = parse_cwd_prompt_hook_value(&args[i])?;
                config.cwd_prompt_hook_from_cli = true;
            }
            "--cwd-prompt-hook" => {
                if i + 1 < args.len() {
                    if let Some(value) = try_parse_cwd_prompt_hook_value(&args[i + 1])? {
                        config.cwd_prompt_hook = value;
                        i += 1;
                    } else {
                        config.cwd_prompt_hook = true;
                    }
                } else {
                    config.cwd_prompt_hook = true;
                }
                config.cwd_prompt_hook_from_cli = true;
            }
            "--no-cwd-prompt-hook" => {
                config.cwd_prompt_hook = false;
                config.cwd_prompt_hook_from_cli = true;
            }
            arg if arg.starts_with("--cwd-prompt-hook=") => {
                let value = arg
                    .split_once('=')
                    .map(|(_, value)| value)
                    .unwrap_or_default();
                config.cwd_prompt_hook = parse_cwd_prompt_hook_value(value)?;
                config.cwd_prompt_hook_from_cli = true;
            }
            arg => {
                return Err(format!("Unknown argument: {}. Use -h for help.", arg));
            }
        }
        i += 1;
    }

    Ok(config)
}

fn parse_cwd_prompt_hook_value(value: &str) -> Result<bool, String> {
    try_parse_cwd_prompt_hook_value(value)?.ok_or_else(|| {
        format!(
            "Invalid cwd prompt hook value: {}. Expected on or off.",
            value
        )
    })
}

fn try_parse_cwd_prompt_hook_value(value: &str) -> Result<Option<bool>, String> {
    match value.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "yes" | "enable" | "enabled" => Ok(Some(true)),
        "off" | "false" | "0" | "no" | "disable" | "disabled" => Ok(Some(false)),
        value if value.starts_with('-') => Ok(None),
        _ => Err(format!(
            "Invalid cwd prompt hook value: {}. Expected on or off.",
            value
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_cwd_prompt_hook_value, try_parse_cwd_prompt_hook_value};

    #[test]
    fn parses_cwd_prompt_hook_on_values() {
        for value in ["on", "true", "1", "yes", "enable", "enabled"] {
            assert!(parse_cwd_prompt_hook_value(value).unwrap());
        }
    }

    #[test]
    fn parses_cwd_prompt_hook_off_values() {
        for value in ["off", "false", "0", "no", "disable", "disabled"] {
            assert!(!parse_cwd_prompt_hook_value(value).unwrap());
        }
    }

    #[test]
    fn leaves_next_option_for_cwd_prompt_hook_flag() {
        assert_eq!(try_parse_cwd_prompt_hook_value("-c").unwrap(), None);
    }
}

/// Check if running inside Windows Terminal
#[cfg(windows)]
fn is_windows_terminal() -> bool {
    // Check for WT_SESSION environment variable (set by Windows Terminal)
    env::var("WT_SESSION").is_ok()
}

/// Detect the host terminal environment
#[cfg(not(windows))]
fn detect_terminal_env() -> String {
    // TERM_PROGRAM is set by most macOS terminals and several cross-platform ones
    if let Ok(program) = env::var("TERM_PROGRAM") {
        let name = match program.as_str() {
            "Apple_Terminal" => "Terminal.app",
            "iTerm.app" => "iTerm2",
            "WezTerm" => "WezTerm",
            "vscode" => "VSCode Terminal",
            "ghostty" => "Ghostty",
            other => other,
        };
        return name.to_string();
    }

    if env::var("ALACRITTY_WINDOW_ID").is_ok() || env::var("ALACRITTY_SOCKET").is_ok() {
        return "Alacritty".to_string();
    }
    if env::var("KITTY_WINDOW_ID").is_ok() {
        return "kitty".to_string();
    }
    if env::var("KONSOLE_VERSION").is_ok() {
        return "Konsole".to_string();
    }
    if env::var("GNOME_TERMINAL_SCREEN").is_ok() {
        return "GNOME Terminal".to_string();
    }
    if env::var("TMUX").is_ok() {
        return "tmux".to_string();
    }

    env::var("TERM").unwrap_or_else(|_| "Unknown".to_string())
}

/// Detect the host terminal environment
#[cfg(windows)]
fn detect_terminal_env() -> String {
    // Check Windows Terminal
    if env::var("WT_SESSION").is_ok() {
        return "Windows Terminal".to_string();
    }
    
    // Check VSCode terminal
    if env::var("VSCODE_INJECTION").is_ok() || env::var("TERM_PROGRAM").map(|v| v == "vscode").unwrap_or(false) {
        return "VSCode Terminal".to_string();
    }
    
    // Check ConEmu
    if env::var("ConEmuPID").is_ok() {
        return "ConEmu".to_string();
    }
    
    // Check Cmder
    if env::var("CMDER_ROOT").is_ok() {
        return "Cmder".to_string();
    }
    
    // Check Hyper
    if env::var("TERM_PROGRAM").map(|v| v == "Hyper").unwrap_or(false) {
        return "Hyper".to_string();
    }
    
    // Check Alacritty
    if env::var("ALACRITTY_LOG").is_ok() || env::var("ALACRITTY_SOCKET").is_ok() {
        return "Alacritty".to_string();
    }
    
    // Check mintty (Git Bash, Cygwin, MSYS2)
    if env::var("MSYSTEM").is_ok() {
        return "MSYS2/MinGW".to_string();
    }
    
    // Default: native console
    "Windows Console".to_string()
}

/// Default shell when neither the CLI nor config.toml specifies one
#[cfg(windows)]
fn default_shell() -> String {
    "cmd.exe".to_string()
}

/// Default shell when neither the CLI nor config.toml specifies one
#[cfg(not(windows))]
fn default_shell() -> String {
    env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

/// Get shell name from command
fn get_shell_name(shell_cmd: &str) -> &str {
    if shell_cmd.contains("pwsh") {
        "PowerShell 7"
    } else if shell_cmd.contains("powershell") {
        "Windows PowerShell"
    } else if shell_cmd.contains("wsl") {
        "WSL"
    } else if shell_cmd.contains("cmd") {
        "Command Prompt"
    } else if shell_cmd.contains("bash") {
        "Bash"
    } else if shell_cmd.contains("zsh") {
        "Zsh"
    } else if shell_cmd.contains("fish") {
        "Fish"
    } else {
        shell_cmd
    }
}

/// Apply font configuration.
///
/// wtmux runs inside a host terminal (Windows Terminal) and cannot change
/// the font renderer directly at runtime.
///
/// **Why we do NOT send OSC 50:**
/// Windows Terminal implements OSC 50 and will switch to the named font.
/// If the specified font is a standard (non-Nerd-Font) variant that lacks
/// Private Use Area glyphs (U+E000–F8FF), Powerline separators and icons
/// fall back to a glyph-less system font and display as boxes or wrong
/// characters.  Sending OSC 50 therefore causes the very problem the user
/// is trying to fix.
///
/// **How to configure the font instead:**
/// Set the font in Windows Terminal's profile settings
/// (`settings.json` → `profiles.defaults.font.face`).  Use a Nerd Font
/// variant (e.g. "CaskaydiaCove Nerd Font", "FiraCode Nerd Font") so that
/// all PUA glyphs are available from the same font face.
///
/// The `[font]` section in wtmux's config.toml is kept for two purposes:
///   1. `suppress_bold` — prevents the OS from substituting a non-Nerd-Font
///      Bold face when the Nerd Font family lacks a bold variant.
///   2. Documentation / future use.
fn apply_font_config(_font: &crate::config::FontConfig) {
    // Intentionally empty — see doc-comment above.
    // OSC 50 is NOT sent because Windows Terminal will switch to the named
    // font, which breaks Nerd Font / Powerline rendering when the specified
    // font lacks PUA glyphs.
}

/// Reset cursor shape to default block cursor
fn reset_cursor_shape() {
    let mut stdout = std::io::stdout();
    let _ = execute!(stdout, SetCursorStyle::SteadyBlock);
}

/// Get encoding name
fn get_encoding_name(codepage: Option<u32>) -> &'static str {
    match codepage {
        Some(65001) => "UTF-8",
        Some(932) => "Shift-JIS",
        Some(cp) => {
            // Return a static str for common codepages
            match cp {
                20932 => "EUC-JP",
                50220 => "ISO-2022-JP",
                _ => "Custom"
            }
        }
        None => "Shift-JIS"
    }
}

/// Relaunch in a native cmd.exe window
#[cfg(windows)]
fn relaunch_in_cmd() -> ! {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Threading::{
        CreateProcessW, STARTUPINFOW, PROCESS_INFORMATION,
        CREATE_NEW_CONSOLE, NORMAL_PRIORITY_CLASS,
    };
    use windows::Win32::Foundation::CloseHandle;
    use windows::core::PWSTR;
    
    // Get current executable path
    let exe = env::current_exe().expect("Failed to get current exe path");
    let exe_str = exe.to_string_lossy();
    
    // Get current arguments, add --no-relaunch to prevent infinite loop
    let args: Vec<String> = env::args().skip(1).collect();
    let mut new_args = vec!["--no-relaunch".to_string()];
    new_args.extend(args.into_iter().filter(|a| a != "-n" && a != "--native"));
    
    // Build command line: "exe_path" arg1 arg2 ...
    let cmd_line = if new_args.is_empty() {
        format!("\"{}\"", exe_str)
    } else {
        format!("\"{}\" {}", exe_str, new_args.join(" "))
    };
    
    // Convert to wide string
    let mut cmd_wide: Vec<u16> = OsStr::new(&cmd_line)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    
    // Build environment block without WT_SESSION
    // Format: VAR1=VALUE1\0VAR2=VALUE2\0...\0\0
    let mut env_block: Vec<u16> = Vec::new();
    for (key, value) in env::vars() {
        // Skip WT_SESSION to ensure the new process doesn't think it's in Windows Terminal
        if key == "WT_SESSION" || key == "WT_PROFILE_ID" || key == "WSLENV" {
            continue;
        }
        let entry = format!("{}={}", key, value);
        env_block.extend(OsStr::new(&entry).encode_wide());
        env_block.push(0);
    }
    env_block.push(0); // Double null terminator
    
    unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        
        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        
        let result = CreateProcessW(
            None,                                    // Application name (use command line)
            PWSTR(cmd_wide.as_mut_ptr()),           // Command line
            None,                                    // Process security attributes
            None,                                    // Thread security attributes
            false,                                   // Inherit handles
            CREATE_NEW_CONSOLE | NORMAL_PRIORITY_CLASS, // Creation flags
            Some(env_block.as_ptr() as *const _),   // Environment (without WT_SESSION)
            None,                                    // Current directory
            &si,                                     // Startup info
            &mut pi,                                 // Process information
        );
        
        if result.is_ok() {
            // Close handles we don't need
            let _ = CloseHandle(pi.hProcess);
            let _ = CloseHandle(pi.hThread);
        }
    }
    
    std::process::exit(0);
}

/// Allocate a new console if running in Windows Terminal
#[cfg(windows)]
fn ensure_native_console() -> bool {
    use windows::Win32::System::Console::{
        AllocConsole, FreeConsole,
    };
    
    // Check if we're in Windows Terminal
    if !is_windows_terminal() {
        return false;
    }
    
    unsafe {
        // Free the current console (Windows Terminal's)
        let _ = FreeConsole();
        
        // Allocate a new native console
        if AllocConsole().is_ok() {
            return true;
        }
    }
    
    false
}

fn main() -> anyhow::Result<()> {
    // Check for -n/--native flag early (before full parsing)
    let args: Vec<String> = env::args().collect();
    if tmux_compat::maybe_run_tmux_compat_cli(&args)? {
        return Ok(());
    }

    #[cfg(windows)]
    let wants_native = args.iter().any(|a| a == "-n" || a == "--native");
    #[cfg(windows)]
    let no_relaunch = args.iter().any(|a| a == "--no-relaunch");


    // If -n flag and running in Windows Terminal, relaunch in native console
    #[cfg(windows)]
    if wants_native && !no_relaunch && is_windows_terminal() {
        // Try to allocate a new console first
        if ensure_native_console() {
            // Successfully got a native console, continue
            eprintln!("Switched to native console for mouse support");
        } else {
            // Fall back to relaunching in a new window
            eprintln!("Detected Windows Terminal, relaunching in native console...");
            relaunch_in_cmd();
        }
    }
    
    // Parse command line arguments
    let config = match parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: {}", e);
            eprintln!("Use --help for usage information");
            std::process::exit(1);
        }
    };

    // Refuse to nest. Panes inherit WTMUX=1, so its presence means this
    // process was started inside a wtmux pane; a nested instance would
    // fight over the prefix key and stack ConPTY inside ConPTY. CLI
    // subcommands (send-keys, list-keys, ...) stay usable — they were
    // dispatched above. Like tmux, unsetting the variable forces it.
    if env::var_os("WTMUX").is_some() {
        eprintln!("Error: wtmux is already running in this terminal (WTMUX is set).");
        eprintln!("Nested wtmux sessions are not supported.");
        #[cfg(windows)]
        eprintln!("To force a nested session: Remove-Item Env:WTMUX (or `set WTMUX=` in cmd), then run wtmux again.");
        #[cfg(not(windows))]
        eprintln!("To force a nested session: unset WTMUX, then run wtmux again.");
        std::process::exit(1);
    }

    // Initialize logging only when --debug is specified
    if config.debug {
        if let Some(wtmux_dir) = crate::config::get_data_dir() {
            let log_path = wtmux_dir.join("wtmux.log");
            
            // Create log directory if needed
            if let Some(parent) = log_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            
            // Open log file (append mode)
            if let Ok(file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
            {
                let subscriber = FmtSubscriber::builder()
                    .with_max_level(Level::DEBUG)
                    .with_writer(std::sync::Mutex::new(file))
                    .with_ansi(false)
                    .finish();
                let _ = tracing::subscriber::set_global_default(subscriber);
                info!("wtmux starting (debug mode)...");
                info!("Log file: {:?}", log_path);
            }
        }
    }
    
    // Set environment variable so child processes can detect wtmux
    env::set_var("WTMUX", "1");
    env::set_var("WTMUX_VERSION", env!("CARGO_PKG_VERSION"));
    // Lets tools inside panes (e.g. `wtmux report-state`) address this instance
    env::set_var("WTMUX_PID", std::process::id().to_string());

    run_terminal(config)?;

    Ok(())
}

/// Run the terminal
fn run_terminal(mut config: Config) -> anyhow::Result<()> {
    use crossterm::terminal;
    
    // Load wtmux config file
    let wtmux_config = WtmuxConfig::load();
    
    // Merge config: command line args override config file
    // Only use config file shell if not explicitly set via CLI
    if !config.shell_from_cli {
        if let Some(ref shell) = wtmux_config.shell {
            config.shell = Some(shell.clone());
        }
    }
    // Default to the platform shell if still not set
    if config.shell.is_none() {
        config.shell = Some(default_shell());
    }
    
    // Codepage from config file (CLI always overrides since it has default)
    // Note: codepage is always Some due to default, so we check wtmux_config
    if let Some(cp) = wtmux_config.codepage {
        // Only override if CLI didn't explicitly set a different value
        // For now, config file codepage is not applied (CLI default takes precedence)
        let _ = cp; // Suppress unused warning
    }

    if !config.cwd_prompt_hook_from_cli {
        config.cwd_prompt_hook = wtmux_config.cwd_prompt_hook;
    }
    
    let keybindings = ParsedKeyBindings::from_config(&wtmux_config.keybindings);

    // Detect terminal environment
    let terminal_env = detect_terminal_env();
    let shell_cmd_str = config.shell.clone().unwrap_or_else(default_shell);
    let shell_name = get_shell_name(&shell_cmd_str);
    let encoding_name = get_encoding_name(config.codepage);
    
    // Log environment info
    info!("Host terminal: {}", terminal_env);
    info!("Shell: {} ({})", shell_name, shell_cmd_str);
    info!("Encoding: {}", encoding_name);
    info!("Multi-pane mode: {}", config.multipane);
    
    // Get terminal size. Some ptys (CI harnesses, expect) report 0x0;
    // fall back to the classic 80x24 instead of panicking on empty grids.
    let (cols, rows) = Renderer::size()?;
    let (cols, rows) = if cols == 0 || rows == 0 {
        (80, 24)
    } else {
        (cols, rows)
    };
    info!("Terminal size: {}x{}", cols, rows);

    if config.multipane {
        // Multi-pane mode
        return run_terminal_wm(
            config,
            cols,
            rows,
            shell_name,
            encoding_name,
            &terminal_env,
            wtmux_config,
            keybindings,
        );
    }

    // Simple single-pane mode
    // Create session (ConPTY always outputs UTF-8)
    let mut session = Session::new(1, cols, rows);

    // Start shell with optional codepage
    if let Err(e) = session.start_with_options(
        Some(&shell_cmd_str),
        config.codepage,
        config.cwd_prompt_hook,
    ) {
        error!("Failed to start shell: {}", e);
        return Err(e.into());
    }

    // Initialize renderer and run with guaranteed cleanup
    let mut renderer = Renderer::new();
    renderer.init()?;
    
    // Set window title with environment info
    let title = format!("wtmux - {} | {} | {}", shell_name, encoding_name, terminal_env);
    print!("\x1b]0;{}\x07", title);
    let _ = std::io::stdout().flush();

    // Run main loop
    let result = run_main_loop(&mut session, &mut renderer, keybindings);

    // Cleanup - multiple attempts to ensure it works
    let _ = renderer.cleanup();
    
    // Force disable raw mode again just to be sure
    let _ = terminal::disable_raw_mode();
    
    // Reset console using escape sequences directly
    print!("\x1b[?1049l"); // Leave alternate screen
    print!("\x1b[?25h");   // Show cursor
    print!("\x1b[0m");     // Reset attributes
    let _ = std::io::stdout().flush();
    
    result
}

/// Run terminal in multi-pane mode
fn run_terminal_wm(
    config: Config,
    cols: u16,
    rows: u16,
    shell_name: &str,
    encoding_name: &str,
    terminal_env: &str,
    wtmux_config: WtmuxConfig,
    keybindings: ParsedKeyBindings,
) -> anyhow::Result<()> {
    use crossterm::terminal;
    use crate::ui::WmRenderer;
    
    // Get color scheme from config
    let color_scheme = wtmux_config.get_color_scheme();
    
    // Parse prefix key from config
    let prefix_key = crate::config::PrefixKey::parse(&wtmux_config.prefix_key)
        .unwrap_or(crate::config::PrefixKey { char: 'b' });

    // Built-in bindings overlaid with [bind] / [bind_root] / unbind. Bad
    // entries are reported and skipped so a typo never costs the whole table.
    let binds = BindTable::build(
        prefix_key.char,
        &wtmux_config.bind,
        &wtmux_config.bind_root,
        &wtmux_config.unbind,
    );
    for error in binds.errors() {
        eprintln!("[wtmux] {error}");
    }


    // Create window manager
    let mut wm = WindowManager::new(
        cols,
        rows,
        config.shell.clone(),
        config.codepage,
        prefix_key,
        config.cwd_prompt_hook,
    );

    // Pane activity monitor settings
    wm.activity_monitor = wtmux_config.activity.enabled;
    wm.quiet_threshold = Duration::from_millis(wtmux_config.activity.quiet_threshold_ms);

    // Drop stale report-state files from a previous run with this pid
    tmux_compat::cleanup_agent_state_dir();


    // Start initial session
    if let Err(e) = wm.start() {
        error!("Failed to start session: {}", e);
        return Err(anyhow::anyhow!(e));
    }
    
    // Force resize to ensure PTY has correct size
    wm.resize(cols, rows);

    // Enable VT byte trace if requested (--vt-trace)
    if config.vt_trace {
        if let Some(dir) = crate::config::get_data_dir() {
            let trace_path = dir.join("vt_trace.log");
            if let Some(session) = wm.get_active_session_mut() {
                match session.enable_vt_trace(&trace_path) {
                    Ok(_) => {
                        eprintln!("[wtmux] VT trace enabled → {:?}", trace_path);
                    }
                    Err(e) => {
                        eprintln!("[wtmux] VT trace failed to open {:?}: {}", trace_path, e);
                    }
                }
            }
        }
    }

    // Initialize renderer with color scheme
    let mut renderer = WmRenderer::with_color_scheme(color_scheme);
    renderer.set_keybindings(&keybindings);
    // Propagate font config into renderer
    renderer.suppress_bold = wtmux_config.font.suppress_bold;
    renderer.init()?;
    
    // Apply font settings from config (best-effort via OSC sequences)
    apply_font_config(&wtmux_config.font);

    // Set window title
    let title = format!("wtmux [Multi] - {} | {} | {}", shell_name, encoding_name, terminal_env);
    print!("\x1b]0;{}\x07", title);
    let _ = std::io::stdout().flush();

    // Run main loop
    let hooks = wtmux_config.hooks.clone();
    let result = run_wm_main_loop(&mut wm, &mut renderer, keybindings, binds, hooks);

    // Cleanup
    let _ = renderer.cleanup();
    let _ = terminal::disable_raw_mode();
    
    print!("\x1b[?1049l");
    print!("\x1b[?25h");
    print!("\x1b[0m");
    let _ = std::io::stdout().flush();
    
    result
}

/// Main event loop for window manager
fn run_wm_main_loop(
    wm: &mut WindowManager,
    renderer: &mut crate::ui::WmRenderer,
    keybindings: ParsedKeyBindings,
    binds: BindTable,
    hooks: crate::config::HooksConfig,
) -> anyhow::Result<()> {
    // Adaptive polling: 10ms while output is flowing, relaxing to 50ms after
    // ~0.5s of idle to cut wake-ups. Input events wake poll() immediately
    // regardless of the timeout, so only the first PTY output after an idle
    // period can be delayed (by at most 50ms).
    let active_poll = Duration::from_millis(10);
    let idle_poll = Duration::from_millis(50);
    let mut idle_ticks: u32 = 0;
    let mut status_publisher = tmux_compat::StatusPublisher::default();
    let theme_list = ColorScheme::list();
    let mut ui = WmAppState::new();
    let pane_numbers_duration = Duration::from_secs(2);

    // Resize debounce: buffer rapid resize events and apply after 30ms of calm.
    // Windows fires one resize event per pixel during drag; without debouncing,
    // each event triggers a full redraw and PTY resize which causes flicker.
    let mut pending_resize: Option<(u16, u16)> = None;
    let mut last_resize_time = std::time::Instant::now();
    let resize_debounce = Duration::from_millis(30);

    // Drives the WORKING spinner while the agent dashboard is open
    let spinner_interval =
        Duration::from_millis(crate::wm::pane::WORKING_SPINNER_INTERVAL_MS);
    let mut last_spinner_tick = std::time::Instant::now();

    // Agent-state plumbing: poll `wtmux report-state` drops at a low rate and
    // dispatch [hooks] commands on state transitions.
    let hooks_enabled = hooks.any_configured();
    let state_report_poll = Duration::from_millis(200);
    let mut last_state_report_poll = std::time::Instant::now();

    // Focus reporting (DECSET 1004): panes that enabled it get CSI I / CSI O
    // when wtmux's pane focus moves, like tmux's focus-events. Host terminal
    // focus is forwarded in the event handler below.
    let mut last_focus = wm.focused_pane_id();

    loop {
        // Check if any session is still running
        if !wm.is_running() {
            info!("All sessions ended");
            break;
        }
        
        // Flush debounced resize after the quiet period has elapsed
        if let Some((cols, rows)) = pending_resize {
            if last_resize_time.elapsed() >= resize_debounce {
                wm.resize(cols, rows);
                if let Some(popup) = ui.popup.as_mut() {
                    let (x, y, w, h) = popup_geometry(cols, rows);
                    popup.apply_geometry(x, y, w, h, crate::wm::BorderStyle::Single);
                }
                ui.render(renderer, wm, &theme_list)?;
                wm.clear_all_dirty();
                pending_resize = None;
            }
        }

        // Check pane numbers timeout
        if ui.mode == UiMode::PaneNumbers
            && ui.pane_numbers_started.elapsed() >= pane_numbers_duration
        {
            ui.close_mode();
            wm.force_full_redraw();
            renderer.render(wm)?;
            wm.clear_all_dirty();
        }

        // Process output and closed panes/tabs.
        let mut needs_render = wm.process_output();

        // OSC 52: a child asked to set the host clipboard
        if let Some(text) = wm.take_osc52() {
            let _ = copy_to_clipboard(&text);
        }

        // Focus reporting: tell panes when wtmux's focus moved between them
        let current_focus = wm.focused_pane_id();
        if current_focus != last_focus {
            if let Some((tab_id, pane_id)) = last_focus {
                wm.notify_pane_focus(tab_id, pane_id, false);
            }
            if let Some((tab_id, pane_id)) = current_focus {
                wm.notify_pane_focus(tab_id, pane_id, true);
            }
            last_focus = current_focus;
        }

        // Popup pane output / lifecycle
        if let Some(popup) = ui.popup.as_mut() {
            if popup.session.process_output().unwrap_or(false) && ui.mode == UiMode::Popup {
                needs_render = true;
            }
            if let Some(text) = popup.session.state.osc52.take() {
                let _ = copy_to_clipboard(&text);
            }
            if !popup.session.is_running() {
                if ui.popup_hold {
                    // display-popup without -E: keep the output visible and
                    // mark the title; any key closes it (key handler below)
                    const EXITED_SUFFIX: &str = " [exited]";
                    if let Some(title) = popup.title.as_mut() {
                        if !title.ends_with(EXITED_SUFFIX) {
                            title.push_str(EXITED_SUFFIX);
                            needs_render = true;
                        }
                    }
                } else {
                    ui.close_popup();
                    wm.force_full_redraw();
                    needs_render = true;
                }
            }
        }

        // Erase an expired transient status message
        if renderer.tick_status_message() {
            wm.force_full_redraw();
            needs_render = true;
        }

        // External IPC: agent states from `wtmux report-state`, and
        // send-keys / capture-pane / display-popup requests
        if last_state_report_poll.elapsed() >= state_report_poll {
            last_state_report_poll = std::time::Instant::now();
            for (tab_id, pane_id, state) in tmux_compat::drain_reported_states() {
                if wm.apply_reported_state(tab_id, pane_id, state) {
                    needs_render = true;
                }
            }

            for req in tmux_compat::drain_requests() {
                let result: Result<Option<String>, String> = match req.command.as_str() {
                    "send-keys" => wm
                        .resolve_target_pane(req.target.as_deref())
                        .and_then(|(tab, pane)| {
                            wm.write_to_pane(
                                tab,
                                pane,
                                &tmux_compat::send_keys_to_bytes(&req.args),
                            )
                        })
                        .map(|_| None),
                    "capture-pane" => {
                        let scrollback = req.args.iter().any(|a| a == "scrollback");
                        wm.resolve_target_pane(req.target.as_deref())
                            .and_then(|(tab, pane)| wm.capture_pane_text(tab, pane, scrollback))
                            .map(Some)
                    }
                    "list-agents" => {
                        Ok(Some(tmux_compat::format_agent_lines(&wm.agent_overview())))
                    }
                    "display-popup" => {
                        let auto_close = req.args.iter().any(|a| a == "-E");
                        let command = req.args.iter().find(|a| *a != "-E").cloned();
                        let hold = command.is_some() && !auto_close;
                        open_popup(wm, &mut ui, command.as_deref(), hold).map(|_| {
                            needs_render = true;
                            None
                        })
                    }
                    other => Err(format!("unsupported request: {other}")),
                };
                tmux_compat::write_reply(&req.id, &result);
            }
        }

        // Fire [hooks] commands on agent state transitions
        if hooks_enabled && wm.activity_monitor {
            let events = wm.drain_agent_state_events();
            if !events.is_empty() {
                run_agent_state_hooks(&hooks, &events);
            }
        }

        if needs_render {
            idle_ticks = 0;
        } else {
            idle_ticks = idle_ticks.saturating_add(1);
        }

        if let Some(snapshot) = wm.tmux_active_pane_snapshot() {
            status_publisher.publish(&snapshot);
        }
        
        // Check again after processing output (panes may have exited)
        if !wm.is_running() {
            info!("All sessions ended after output processing");
            break;
        }
        
        // Render based on current mode
        if ui.defers_background_render() {
            // In copy mode, rename mode, or context menu, only render on key events
            // (rendering happens in the key handler below)
        } else if needs_render {
            ui.render(renderer, wm, &theme_list)?;
            // Clear dirty state after rendering so the next frame only redraws
            // rows that have genuinely changed.
            wm.clear_all_dirty();
        } else if ui.mode == UiMode::AgentDashboard
            && last_spinner_tick.elapsed() >= spinner_interval
        {
            // Keep the WORKING spinner animating while the dashboard is open
            last_spinner_tick = std::time::Instant::now();
            ui.render(renderer, wm, &theme_list)?;
            wm.clear_all_dirty();
        }

        // Poll for events
        let poll_timeout = if idle_ticks > 50 { idle_poll } else { active_poll };
        if input::poll(poll_timeout)? {
            idle_ticks = 0;
            match input::read()? {
                Event::Key(key_event) => {
                    if key_event.kind != KeyEventKind::Press {
                        // Kitty keyboard protocol (report event types):
                        // forward key releases to the pane that requested
                        // them. Releases never trigger wtmux keybindings
                        // or modal UI.
                        if key_event.kind == crossterm::event::KeyEventKind::Release {
                            if ui.mode == UiMode::Popup {
                                let bytes = ui.popup.as_ref().and_then(|popup| {
                                    KeyMapper::map_for_pane(&key_event, &popup.session.state)
                                });
                                if let (Some(bytes), Some(popup)) = (bytes, ui.popup.as_mut()) {
                                    let _ = popup.session.write(&bytes);
                                }
                            } else if ui.mode == UiMode::Normal && !wm.prefix_mode {
                                let bytes = wm.focused_state().and_then(|state| {
                                    KeyMapper::map_for_pane(&key_event, state)
                                });
                                if let Some(bytes) = bytes {
                                    let _ = wm.write(&bytes);
                                }
                            }
                        }
                        continue;
                    }

                    // ── Popup mode: all input goes to the popup pane ─────────
                    // Safety valve: Prefix, x kills a stuck popup.
                    if ui.mode == UiMode::Popup {
                        // A held popup whose process has exited: scroll keys
                        // browse the output, any other key closes it
                        if ui
                            .popup
                            .as_ref()
                            .is_some_and(|popup| !popup.session.is_running())
                        {
                            let mut close = false;
                            if let Some(popup) = ui.popup.as_mut() {
                                let page = popup.inner_size().1.max(1) as usize;
                                let screen = popup.session.state.active_screen_mut();
                                match key_event.code {
                                    KeyCode::Up => screen.scroll_view_up(1),
                                    KeyCode::Down => screen.scroll_view_down(1),
                                    KeyCode::PageUp => screen.scroll_view_up(page),
                                    KeyCode::PageDown => screen.scroll_view_down(page),
                                    KeyCode::Home => screen.scroll_view_up(usize::MAX / 2),
                                    KeyCode::End => screen.scroll_to_bottom(),
                                    _ => close = true,
                                }
                            }
                            if close {
                                ui.close_popup();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                            } else {
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            wm.clear_all_dirty();
                            continue;
                        }
                        let prefix_pressed = key_event
                            .modifiers
                            .contains(KeyModifiers::CONTROL)
                            && key_event.code == KeyCode::Char(wm.prefix_key.char);
                        if ui.popup_prefix {
                            ui.popup_prefix = false;
                            if key_event.code == KeyCode::Char('x') {
                                ui.close_popup();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                                wm.clear_all_dirty();
                                continue;
                            }
                            // Any other key falls through to the popup
                        } else if prefix_pressed {
                            ui.popup_prefix = true;
                            continue;
                        }
                        if let Some(popup) = ui.popup.as_mut() {
                            let bytes = KeyMapper::map_for_pane(&key_event, &popup.session.state)
                                .unwrap_or_default();
                            if !bytes.is_empty() {
                                // Typing returns the view to the live bottom,
                                // like regular panes
                                popup.session.state.active_screen_mut().scroll_to_bottom();
                                let _ = popup.session.write(&bytes);
                            }
                        }
                        continue;
                    }

                    // ── Command prompt (Prefix + :) ──────────────────────────
                    if ui.mode == UiMode::CommandPrompt {
                        match key_event.code {
                            KeyCode::Esc => {
                                ui.close_mode();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                                wm.clear_all_dirty();
                            }
                            KeyCode::Enter => {
                                let line = ui.command_buffer.trim().to_string();
                                ui.close_mode();
                                wm.force_full_redraw();
                                if !line.is_empty() {
                                    use crate::command_prompt::PromptAction;
                                    match crate::command_prompt::parse(&line) {
                                        Ok(PromptAction::DisplayPopup { command, hold }) => {
                                            if let Err(e) =
                                                open_popup(wm, &mut ui, command.as_deref(), hold)
                                            {
                                                renderer.set_status_message(format!(
                                                    "display-popup: {e}"
                                                ));
                                            }
                                        }
                                        Ok(action) => {
                                            let message = execute_prompt_action(wm, action);
                                            renderer.set_status_message(message);
                                        }
                                        Err(e) => renderer.set_status_message(e),
                                    }
                                }
                                ui.render(renderer, wm, &theme_list)?;
                                wm.clear_all_dirty();
                            }
                            KeyCode::Backspace => {
                                ui.command_buffer.pop();
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            KeyCode::Char('u')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                ui.command_buffer.clear();
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            KeyCode::Char(c)
                                if !key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                ui.command_buffer.push(c);
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // ── Message composer (Prefix + m) ────────────────────────
                    if ui.mode == UiMode::MessageComposer {
                        match key_event.code {
                            KeyCode::Esc => {
                                ui.close_mode();
                                pop_composer_key_reporting();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                                wm.clear_all_dirty();
                                continue;
                            }
                            // Ctrl+Enter sends; Ctrl+S is the fallback for
                            // terminals that cannot report Ctrl+Enter
                            KeyCode::Enter
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                send_composer_message(wm, &mut ui, renderer);
                                ui.render(renderer, wm, &theme_list)?;
                                wm.clear_all_dirty();
                                continue;
                            }
                            KeyCode::Char('s')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                send_composer_message(wm, &mut ui, renderer);
                                ui.render(renderer, wm, &theme_list)?;
                                wm.clear_all_dirty();
                                continue;
                            }
                            // Enter inserts a newline like a normal editor
                            KeyCode::Enter => {
                                ui.message_composer.insert_newline();
                            }
                            KeyCode::Backspace => ui.message_composer.backspace(),
                            KeyCode::Delete => ui.message_composer.delete(),
                            KeyCode::Left => {
                                if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                                    ui.message_composer.select_left();
                                } else {
                                    ui.message_composer.move_left();
                                }
                            }
                            KeyCode::Right => {
                                if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                                    ui.message_composer.select_right();
                                } else {
                                    ui.message_composer.move_right();
                                }
                            }
                            KeyCode::Up => {
                                if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                                    ui.message_composer.select_up();
                                } else {
                                    ui.message_composer.move_up();
                                }
                            }
                            KeyCode::Down => {
                                if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                                    ui.message_composer.select_down();
                                } else {
                                    ui.message_composer.move_down();
                                }
                            }
                            KeyCode::Home => ui.message_composer.move_home(),
                            KeyCode::End => ui.message_composer.move_end(),
                            KeyCode::Char('u')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                ui.message_composer.clear();
                            }
                            // Sent-message history (readline-style)
                            KeyCode::Char('p')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                ui.message_composer.history_prev();
                            }
                            KeyCode::Char('n')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                ui.message_composer.history_next();
                            }
                            // Ctrl+V: paste from the system clipboard. Needed for
                            // terminals that forward Ctrl+V as a key event instead
                            // of translating it into a bracketed-paste event.
                            KeyCode::Char('v')
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                if let Some(text) = read_clipboard_text() {
                                    ui.message_composer.insert_str(&text);
                                }
                            }
                            KeyCode::Char(c)
                                if !key_event.modifiers.contains(KeyModifiers::CONTROL) =>
                            {
                                ui.message_composer.insert_char(c);
                            }
                            _ => {}
                        }
                        ui.render(renderer, wm, &theme_list)?;
                        continue;
                    }

                    // Handle context menu keyboard navigation
                    if ui.mode == UiMode::ContextMenu {
                        match key_event.code {
                            KeyCode::Esc => {
                                ui.close_mode();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                ui.context_menu.up();
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                ui.context_menu.down();
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                let menu_action = ui.context_menu.selected_action();
                                ui.close_mode();
                                wm.force_full_redraw();
                                if menu_action == ContextMenuAction::RenamePane {
                                    open_rename_pane(&mut ui, wm);
                                    ui.render(renderer, wm, &theme_list)?;
                                } else {
                                    apply_app_action(wm, menu_action.into());
                                    renderer.render(wm)?;
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    
                    // Handle copy mode
                    if ui.mode == UiMode::CopyMode {
                        let mut needs_full_redraw = false;
                        let old_scroll = ui.copy_mode.scroll_offset;
                        
                        if ui.copy_mode.search_mode {
                            // Search input mode
                            needs_full_redraw = true;
                            match key_event.code {
                                KeyCode::Esc => {
                                    ui.copy_mode.cancel_search();
                                }
                                KeyCode::Enter => {
                                    ui.copy_mode.execute_search(wm);
                                }
                                KeyCode::Backspace => {
                                    ui.copy_mode.search_backspace();
                                }
                                KeyCode::Char(c) => {
                                    ui.copy_mode.search_input(c);
                                }
                                _ => {}
                            }
                        } else {
                            // Normal copy mode
                            match key_event.code {
                                // Exit copy mode
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    ui.close_mode();
                                    renderer.render(wm)?;
                                    continue;
                                }
                                // Movement - vim style (cursor only update unless scroll changes)
                                KeyCode::Char('h') | KeyCode::Left => {
                                    ui.copy_mode.cursor_left(wm);
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    ui.copy_mode.cursor_down(wm);
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    ui.copy_mode.cursor_up(wm);
                                }
                                KeyCode::Char('l') | KeyCode::Right => {
                                    ui.copy_mode.cursor_right(wm);
                                }
                                // Line navigation
                                KeyCode::Char('0') => {
                                    ui.copy_mode.line_start();
                                }
                                KeyCode::Char('$') => {
                                    ui.copy_mode.line_end(wm);
                                }
                                // Page navigation - needs full redraw
                                KeyCode::PageUp | KeyCode::Char('b') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    ui.copy_mode.page_up(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::PageDown | KeyCode::Char('f') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    ui.copy_mode.page_down(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    ui.copy_mode.half_page_up(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    ui.copy_mode.half_page_down(wm);
                                    needs_full_redraw = true;
                                }
                                // Go to top/bottom - needs full redraw
                                KeyCode::Char('g') => {
                                    ui.copy_mode.goto_top(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('G') => {
                                    ui.copy_mode.goto_bottom(wm);
                                    needs_full_redraw = true;
                                }
                                // Selection - needs full redraw
                                KeyCode::Char(' ') | KeyCode::Char('v') => {
                                    ui.copy_mode.toggle_selection();
                                    needs_full_redraw = true;
                                }
                                // Copy
                                KeyCode::Enter | KeyCode::Char('y') => {
                                    if let Some(text) = ui.copy_mode.copy_selection(wm) {
                                        // Copy to clipboard
                                        let _ = copy_to_clipboard(&text);
                                        ui.close_mode();
                                        renderer.render(wm)?;
                                        continue;
                                    }
                                }
                                // Search - needs full redraw
                                KeyCode::Char('/') => {
                                    ui.copy_mode.enter_search(true);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('?') => {
                                    ui.copy_mode.enter_search(false);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('n') => {
                                    ui.copy_mode.find_next_match(false);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('N') => {
                                    ui.copy_mode.find_prev_match();
                                    needs_full_redraw = true;
                                }
                                _ => {}
                            }
                        }
                        
                        // Check if scroll changed
                        if ui.copy_mode.scroll_offset != old_scroll {
                            needs_full_redraw = true;
                        }
                        
                        // Render
                        if needs_full_redraw || ui.copy_mode.selection_start.is_some() {
                            ui.render(renderer, wm, &theme_list)?;
                        } else {
                            renderer.render_copy_mode_cursor_only(wm, &ui.copy_mode)?;
                        }
                        continue;
                    }
                    
                    // Handle rename mode
                    if ui.mode == UiMode::Rename {
                        match key_event.code {
                            KeyCode::Esc => {
                                ui.close_mode();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                                continue;
                            }
                            KeyCode::Enter => {
                                match ui.rename_target {
                                    RenameTarget::Window => {
                                        if !ui.rename_buffer.is_empty() {
                                            wm.rename_active_tab(&ui.rename_buffer);
                                        }
                                    }
                                    // Empty name restores the default pane title
                                    RenameTarget::Pane => {
                                        wm.rename_focused_pane(&ui.rename_buffer);
                                    }
                                }
                                ui.close_mode();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                                continue;
                            }
                            KeyCode::Backspace => {
                                ui.rename_buffer.pop();
                            }
                            KeyCode::Char(c)
                                if str_display_width(&ui.rename_buffer) + char_width(c) <= 30 =>
                            {
                                ui.rename_buffer.push(c);
                            }
                            _ => {}
                        }
                        ui.render(renderer, wm, &theme_list)?;
                        continue;
                    }
                    
                    // Handle pane numbers mode - select pane by number
                    if ui.mode == UiMode::PaneNumbers {
                        if let KeyCode::Char(c) = key_event.code {
                            if c.is_ascii_digit() {
                                let num = c.to_digit(10).unwrap_or(0) as usize;
                                wm.select_pane_by_number(num);
                                reset_cursor_shape();
                            }
                        }
                        ui.close_mode();
                        wm.force_full_redraw();
                        renderer.render(wm)?;
                        continue;
                    }

                    // Handle window selector mode
                    if ui.mode == UiMode::WindowSelector {
                        let windows = wm.window_info();
                        if ui.window_selector.kill_confirm {
                            // Waiting for kill confirmation (y/N)
                            if matches!(key_event.code, KeyCode::Char('y') | KeyCode::Char('Y')) {
                                match ui.window_selector.selected_entry(&windows) {
                                    Some(TreeEntry::Window { window }) => {
                                        wm.close_tab_at(window);
                                    }
                                    Some(TreeEntry::Pane { window, pane }) => {
                                        wm.close_pane_at(window, pane);
                                    }
                                    None => {}
                                }
                            }
                            ui.window_selector.kill_confirm = false;
                        } else {
                            let entry_count = ui.window_selector.entries(&windows).len();
                            match key_event.code {
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    ui.window_selector.move_up(entry_count);
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    ui.window_selector.move_down(entry_count);
                                }
                                KeyCode::Home => {
                                    ui.window_selector.selected = 0;
                                }
                                KeyCode::End => {
                                    ui.window_selector.selected =
                                        entry_count.saturating_sub(1);
                                }
                                // Expand / collapse the tree (tmux: Right/Left)
                                KeyCode::Right | KeyCode::Char('l') => {
                                    ui.window_selector.expand(&windows);
                                }
                                KeyCode::Left | KeyCode::Char('h') => {
                                    ui.window_selector.collapse(&windows);
                                }
                                // Jump to a window by its display number (1-9)
                                KeyCode::Char(c) if c.is_ascii_digit() => {
                                    let num = c.to_digit(10).unwrap_or(0) as usize;
                                    ui.window_selector.jump_to_window(&windows, num);
                                }
                                // Kill the selected window or pane (tmux: x)
                                KeyCode::Char('x') => {
                                    match ui.window_selector.selected_entry(&windows) {
                                        Some(TreeEntry::Window { .. }) if windows.len() > 1 => {
                                            ui.window_selector.kill_confirm = true;
                                        }
                                        Some(TreeEntry::Pane { window, .. })
                                            if windows[window].panes.len() > 1
                                                || windows.len() > 1 =>
                                        {
                                            ui.window_selector.kill_confirm = true;
                                        }
                                        _ => {}
                                    }
                                }
                                KeyCode::Enter => {
                                    match ui.window_selector.selected_entry(&windows) {
                                        Some(TreeEntry::Window { window }) => {
                                            wm.select_tab_at(window);
                                        }
                                        Some(TreeEntry::Pane { window, pane }) => {
                                            wm.focus_pane_at(window, pane);
                                        }
                                        None => {}
                                    }
                                    reset_cursor_shape();
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                }
                                _ => {}
                            }
                        }
                        if ui.mode == UiMode::WindowSelector {
                            // The window list may have changed (kill); re-clamp
                            let entry_count =
                                ui.window_selector.entries(&wm.window_info()).len();
                            ui.window_selector.clamp(entry_count);
                            ui.render(renderer, wm, &theme_list)?;
                        } else {
                            renderer.render(wm)?;
                        }
                        continue;
                    }

                    // Handle agent dashboard mode
                    if ui.mode == UiMode::AgentDashboard {
                        let entries = wm.agent_overview();
                        match key_event.code {
                            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('g') => {
                                ui.close_mode();
                                wm.force_full_redraw();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                ui.agent_dashboard.move_up(entries.len());
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                ui.agent_dashboard.move_down(entries.len());
                            }
                            KeyCode::Char('a') => {
                                if wm.focus_next_attention() {
                                    reset_cursor_shape();
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                }
                            }
                            KeyCode::Enter => {
                                if let Some(entry) = ui.agent_dashboard.selected_entry(&entries) {
                                    wm.focus_pane_at(entry.window_index, entry.pane_index);
                                    reset_cursor_shape();
                                }
                                ui.close_mode();
                                wm.force_full_redraw();
                            }
                            // Compose a message to the selected pane
                            KeyCode::Char('m') => {
                                if let Some(entry) = ui.agent_dashboard.selected_entry(&entries) {
                                    if let Some((tab_id, pane_id)) =
                                        wm.pane_ids_at(entry.window_index, entry.pane_index)
                                    {
                                        ui.close_mode();
                                        let label = format!(
                                            "{}:{} · {}: {}",
                                            entry.window_number,
                                            entry.window_name,
                                            entry.pane_number,
                                            entry.pane_title
                                        );
                                        ui.message_composer.open(tab_id, pane_id, label);
                                        ui.mode = UiMode::MessageComposer;
                                        push_composer_key_reporting();
                                        wm.force_full_redraw();
                                        ui.render(renderer, wm, &theme_list)?;
                                        continue;
                                    }
                                }
                            }
                            _ => {}
                        }
                        if ui.mode == UiMode::AgentDashboard {
                            ui.agent_dashboard.clamp(wm.agent_overview().len());
                            ui.render(renderer, wm, &theme_list)?;
                        } else {
                            renderer.render(wm)?;
                        }
                        continue;
                    }

                    // Handle theme selector mode
                    if ui.mode == UiMode::ThemeSelector {
                        match key_event.code {
                            KeyCode::Esc => {
                                ui.close_mode();
                                wm.force_full_redraw();
                            }
                            KeyCode::Up => {
                                if ui.theme_selector_index > 0 {
                                    ui.theme_selector_index -= 1;
                                }
                            }
                            KeyCode::Down => {
                                if ui.theme_selector_index + 1 < theme_list.len() {
                                    ui.theme_selector_index += 1;
                                }
                            }
                            KeyCode::Enter => {
                                let scheme_name = theme_list[ui.theme_selector_index];
                                renderer.set_color_scheme(ColorScheme::by_name(scheme_name));
                                ui.close_mode();
                                wm.force_full_redraw();
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                let num = c.to_digit(10).unwrap_or(0) as usize;
                                if num >= 1 && num <= theme_list.len() {
                                    ui.theme_selector_index = num - 1;
                                    let scheme_name = theme_list[ui.theme_selector_index];
                                    renderer.set_color_scheme(ColorScheme::by_name(scheme_name));
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                }
                            }
                            _ => {}
                        }
                        if ui.mode == UiMode::ThemeSelector {
                            ui.render(renderer, wm, &theme_list)?;
                        } else {
                            renderer.render(wm)?;
                        }
                        continue;
                    }

                    // Handle selector mode
                    if ui.mode == UiMode::HistorySelector {
                        {
                            let selector = ui.history_selector_mut();
                            match key_event.code {
                                KeyCode::Esc => {
                                    selector.hide();
                                    wm.force_full_redraw();
                                }
                                KeyCode::Enter => {
                                    if let Some(command) = selector.confirm() {
                                        if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                                            // Shift+Enter: append with && (run if previous succeeds)
                                            let append_cmd = format!(" && {}", command);
                                            let _ = wm.write(append_cmd.as_bytes());
                                        } else if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                                            // Ctrl+Enter: append with & (background/parallel)
                                            let append_cmd = format!(" & {}", command);
                                            let _ = wm.write(append_cmd.as_bytes());
                                        } else {
                                            // Enter: replace current input with history command
                                            wm.clear_current_input();
                                            let _ = wm.write(command.as_bytes());
                                        }
                                    }
                                }
                                KeyCode::Up => {
                                    selector.select_up();
                                }
                                KeyCode::Down => {
                                    selector.select_down();
                                }
                                KeyCode::Backspace => {
                                    selector.backspace();
                                }
                                KeyCode::Delete => {
                                    selector.delete_selected();
                                }
                                KeyCode::Char(c) => {
                                    // Number selection only when query is empty
                                    if selector.query.is_empty() && c.is_ascii_digit() {
                                        if let Some(num) = c.to_digit(10) {
                                            if (1..=9).contains(&num) {
                                                if let Some(command) =
                                                    selector.select_number(num as usize)
                                                {
                                                    // Clear current input and insert
                                                    wm.clear_current_input();
                                                    let _ = wm.write(command.as_bytes());
                                                }
                                            }
                                        }
                                    }
                                    // Add to search query only while still visible.
                                    if selector.visible {
                                        selector.input_char(c);
                                    }
                                }
                                _ => {}
                            }
                        }
                        let selector_visible = ui
                            .history_selector
                            .as_ref()
                            .is_some_and(|selector| selector.visible);
                        if selector_visible {
                            ui.render(renderer, wm, &theme_list)?;
                        } else {
                            ui.mode = UiMode::Normal;
                            renderer.render(wm)?;
                        }
                        continue;
                    }

                    // Handle prefix mode
                    if wm.prefix_mode {
                        wm.prefix_mode = false;
                        let bound = binds
                            .lookup_prefix(&key_event)
                            .unwrap_or(BoundAction::Noop);
                        if apply_ui_action(bound, &mut ui, wm, renderer) {
                            ui.render(renderer, wm, &theme_list)?;
                        } else {
                            if let Some(action) = app_action_for(bound, wm) {
                                apply_app_action(wm, action);
                            }
                            renderer.render(wm)?;
                        }
                        continue;
                    }

                    // Check for prefix key (configurable, default: Ctrl+B)
                    if key_event.modifiers.contains(KeyModifiers::CONTROL)
                        && key_event.code == KeyCode::Char(wm.prefix_key.char)
                    {
                        wm.prefix_mode = true;
                        renderer.render(wm)?;
                        continue;
                    }

                    // Prefix-less bindings from [bind_root]. Checked before
                    // the key reaches the PTY, so a root binding shadows
                    // whatever the shell would have done with that key.
                    if !binds.root_is_empty() {
                        if let Some(bound) = binds.lookup_root(&key_event) {
                            if apply_ui_action(bound, &mut ui, wm, renderer) {
                                ui.render(renderer, wm, &theme_list)?;
                            } else {
                                if let Some(action) = app_action_for(bound, wm) {
                                    apply_app_action(wm, action);
                                }
                                renderer.render(wm)?;
                            }
                            continue;
                        }
                    }

                    if keybindings.history_selector.matches(&key_event)
                        && !wm.is_in_alternate_screen()
                    {
                        ui.close_mode();
                        let selector = ui.history_selector_mut();
                        selector.show();
                        ui.mode = UiMode::HistorySelector;
                        ui.render(renderer, wm, &theme_list)?;
                        continue;
                    }

                    // ── Global [keybindings]: scrollback / selection / copy ────────
                    // Same semantics as the single-pane loop, applied to the
                    // focused pane. Consumed before the key reaches the PTY.
                    if keybindings.scrollback_up.matches(&key_event) {
                        wm.handle_scroll(10);
                        renderer.render(wm)?;
                        continue;
                    }
                    if keybindings.scrollback_down.matches(&key_event) {
                        wm.handle_scroll(-10);
                        renderer.render(wm)?;
                        continue;
                    }
                    if keybindings.scrollback_top.matches(&key_event) {
                        wm.scroll_to_top();
                        renderer.render(wm)?;
                        continue;
                    }
                    if keybindings.scrollback_bottom.matches(&key_event) {
                        wm.scroll_to_bottom();
                        renderer.render(wm)?;
                        continue;
                    }

                    let selection_direction = if keybindings.selection_left.matches(&key_event) {
                        Some(KeyCode::Left)
                    } else if keybindings.selection_right.matches(&key_event) {
                        Some(KeyCode::Right)
                    } else if keybindings.selection_up.matches(&key_event) {
                        Some(KeyCode::Up)
                    } else if keybindings.selection_down.matches(&key_event) {
                        Some(KeyCode::Down)
                    } else {
                        None
                    };
                    if let Some(direction) = selection_direction {
                        if let Some(state) = wm.focused_state_mut() {
                            handle_selection_key(state, direction);
                            state.active_screen_mut().mark_all_dirty();
                        }
                        renderer.render(wm)?;
                        continue;
                    }

                    if keybindings.copy_selection.matches(&key_event) {
                        if let Some(text) = wm
                            .focused_state_mut()
                            .and_then(|state| state.get_selected_text())
                        {
                            if !text.is_empty() {
                                let _ = copy_to_clipboard(&text);
                            }
                        }
                        continue;
                    }

                    // Escape clears an existing selection instead of reaching
                    // the shell.
                    if key_event.code == KeyCode::Esc {
                        let cleared = wm.focused_state_mut().is_some_and(|state| {
                            let had_selection = state.selection.is_some();
                            state.clear_selection();
                            had_selection
                        });
                        if cleared {
                            renderer.render(wm)?;
                            continue;
                        }
                    }

                    // ── Keystroke tracker (cmd.exe fallback for history) ──────────
                    // Update the tracker BEFORE sending the key to the PTY so that
                    // get_current_line() can read the buffer on Enter.
                    if !wm.is_in_alternate_screen() {
                        match key_event.code {
                            KeyCode::Char(ch) => {
                                // Skip control-key combos (Ctrl+C etc.)
                                let is_ctrl = key_event.modifiers
                                    .contains(crossterm::event::KeyModifiers::CONTROL);
                                if !is_ctrl {
                                    wm.keystroke_push_char(ch);
                                }
                            }
                            KeyCode::Backspace => {
                                wm.keystroke_backspace();
                            }
                            _ => {}
                        }
                        // Ctrl+W – delete word
                        if key_event.code == KeyCode::Char('w')
                            && key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            wm.keystroke_delete_word();
                        }
                        // Ctrl+U or Ctrl+C – clear line
                        if (key_event.code == KeyCode::Char('u')
                            || key_event.code == KeyCode::Char('c'))
                            && key_event.modifiers.contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            wm.keystroke_clear();
                        }
                    }

                    // ── History recording on Enter ─────────────────────────────────
                    // get_current_line() uses the priority chain:
                    //   OSC 133/633 confirmed → OSC prompt-end → keystroke buffer → strip_prompt
                    if key_event.code == KeyCode::Enter && !wm.is_in_alternate_screen() {
                        if let Some(command) = wm.get_current_line() {
                            if !command.is_empty() {
                                ui.history_selector_mut().add_to_history(command);
                            }
                        }
                        // Reset keystroke buffer for next command
                        wm.keystroke_clear();
                        // Consume confirmed command so it is not re-used
                        wm.take_confirmed_command();
                    }

                    // Reset scroll to bottom on any key input (return to live
                    // view) and drop any leftover selection, matching the
                    // single-pane loop.
                    wm.scroll_to_bottom();
                    wm.clear_selection();

                    // Send key to focused pane, honoring its terminal modes,
                    // kitty keyboard flags, and win32-input-mode request
                    let bytes = wm
                        .focused_state()
                        .and_then(|state| KeyMapper::map_for_pane(&key_event, state))
                        .unwrap_or_default();
                    if !bytes.is_empty() {
                        let _ = wm.write(&bytes);
                    }
                }

                Event::Mouse(mouse_event) => {
                    use crossterm::event::{MouseEventKind, MouseButton};

                    // Message composer: mouse interaction is not routed
                    if ui.mode == UiMode::MessageComposer {
                        continue;
                    }

                    // Popup: the wheel scrolls its scrollback; other mouse
                    // interaction is not routed
                    if ui.mode == UiMode::Popup {
                        let scrolled = match ui.popup.as_mut() {
                            Some(popup) => {
                                let screen = popup.session.state.active_screen_mut();
                                match mouse_event.kind {
                                    MouseEventKind::ScrollUp => {
                                        screen.scroll_view_up(3);
                                        true
                                    }
                                    MouseEventKind::ScrollDown => {
                                        screen.scroll_view_down(3);
                                        true
                                    }
                                    _ => false,
                                }
                            }
                            None => false,
                        };
                        if scrolled {
                            ui.render(renderer, wm, &theme_list)?;
                        }
                        continue;
                    }

                    // Window selector mouse handling. Plain mouse movement must
                    // not dismiss the popup (any-event tracking reports every
                    // motion); wheel moves the selection, left click chooses a
                    // row, and clicking outside the popup closes it.
                    if ui.mode == UiMode::WindowSelector {
                        let windows = wm.window_info();
                        let entries = ui.window_selector.entries(&windows);
                        match mouse_event.kind {
                            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                                ui.window_selector.kill_confirm = false;
                                if mouse_event.kind == MouseEventKind::ScrollUp {
                                    ui.window_selector.move_up(entries.len());
                                } else {
                                    ui.window_selector.move_down(entries.len());
                                }
                                ui.render(renderer, wm, &theme_list)?;
                            }
                            MouseEventKind::Down(button) => {
                                ui.window_selector.kill_confirm = false;
                                let layout = renderer.window_selector_layout(
                                    wm,
                                    entries.len(),
                                    ui.window_selector.selected,
                                );
                                let clicked_row = layout.as_ref().and_then(|l| {
                                    l.list_row_at(
                                        entries.len(),
                                        mouse_event.column,
                                        mouse_event.row,
                                    )
                                });
                                let inside = layout
                                    .as_ref()
                                    .is_some_and(|l| l.contains(mouse_event.column, mouse_event.row));

                                if button == MouseButton::Left {
                                    if let Some(index) = clicked_row {
                                        match entries.get(index) {
                                            Some(TreeEntry::Window { window }) => {
                                                wm.select_tab_at(*window);
                                            }
                                            Some(TreeEntry::Pane { window, pane }) => {
                                                wm.focus_pane_at(*window, *pane);
                                            }
                                            None => {}
                                        }
                                        reset_cursor_shape();
                                        ui.close_mode();
                                        wm.force_full_redraw();
                                        renderer.render(wm)?;
                                    } else if !inside {
                                        // Click outside the popup closes it
                                        ui.close_mode();
                                        wm.force_full_redraw();
                                        renderer.render(wm)?;
                                    } else {
                                        ui.render(renderer, wm, &theme_list)?;
                                    }
                                } else {
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                    renderer.render(wm)?;
                                }
                            }
                            // Moved / Drag / Up: keep the selector open
                            _ => {}
                        }
                        continue;
                    }
                    
                    // Close snippet selector on mouse click outside.
                    // Moved / Drag / Scroll must not dismiss it.
                    if ui.mode == UiMode::HistorySelector {
                        if matches!(mouse_event.kind, MouseEventKind::Down(_)) {
                            ui.close_mode();
                            wm.force_full_redraw();
                            renderer.render(wm)?;
                        } else {
                            continue;
                        }
                    }
                    
                    // Handle context menu interactions
                    if ui.mode == UiMode::ContextMenu {
                        match mouse_event.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                if let Some(action) = ui
                                    .context_menu
                                    .handle_click(mouse_event.column, mouse_event.row)
                                {
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                    if action == ContextMenuAction::RenamePane {
                                        open_rename_pane(&mut ui, wm);
                                        ui.render(renderer, wm, &theme_list)?;
                                    } else {
                                        apply_app_action(wm, action.into());
                                        renderer.render(wm)?;
                                    }
                                } else {
                                    // Clicked outside menu - close it
                                    ui.close_mode();
                                    wm.force_full_redraw();
                                    renderer.render(wm)?;
                                }
                            }
                            MouseEventKind::Down(MouseButton::Right) => {
                                // Close menu on right click
                                ui.close_mode();
                                wm.force_full_redraw();
                                renderer.render(wm)?;
                            }
                            MouseEventKind::Moved | MouseEventKind::Drag(_)
                                if ui
                                    .context_menu
                                    .update_hover(mouse_event.column, mouse_event.row) =>
                            {
                                // Highlight item under cursor.
                                renderer.render_context_menu_only(&ui.context_menu)?;
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // Other overlays are keyboard-driven. A mouse click
                    // dismisses them as one state transition instead of
                    // leaving hidden modal state active behind the scene.
                    // Moved / Drag / Scroll events are ignored so that
                    // merely moving the pointer does not close the overlay.
                    if ui.mode != UiMode::Normal {
                        if matches!(mouse_event.kind, MouseEventKind::Down(_)) {
                            ui.close_mode();
                            wm.prefix_mode = false;
                            wm.force_full_redraw();
                            renderer.render(wm)?;
                        }
                        continue;
                    }

                    let split_resize_mouse_event = match mouse_event.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            wm.is_split_resize_target(mouse_event.column, mouse_event.row)
                        }
                        MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left) => {
                            wm.is_resizing_split()
                        }
                        _ => false,
                    };

                    if split_resize_mouse_event {
                        match mouse_event.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                wm.handle_mouse_down(mouse_event.column, mouse_event.row);
                            }
                            MouseEventKind::Drag(MouseButton::Left) => {
                                wm.handle_mouse_drag(mouse_event.column, mouse_event.row);
                            }
                            MouseEventKind::Up(MouseButton::Left) => {
                                let _ = wm.handle_mouse_up();
                            }
                            _ => {}
                        }
                        renderer.render(wm)?;
                        continue;
                    }
                    
                    // Check for mouse passthrough to child application
                    // Shift key bypasses passthrough for wtmux's own text selection
                    let shift_held = mouse_event.modifiers.contains(KeyModifiers::SHIFT);
                    
                    // Determine if this event should be passed to child app:
                    // 1. Child app has enabled mouse tracking (DECSET 1000/1002/1003)
                    // 2. Shift is not being held (Shift = force wtmux handling)
                    // 3. Event is within the pane content area (not tab bar/status bar)
                    if !shift_held && wm.focused_pane_wants_mouse() {
                        // Check if event is in content area (not tab bar or status bar)
                        let in_content_area = mouse_event.row >= wm.tab_bar_height 
                            && mouse_event.row < wm.height.saturating_sub(wm.status_bar_height);
                        
                        if in_content_area {
                            // Convert to content-area relative coordinates
                            let content_y = mouse_event.row - wm.tab_bar_height;
                            
                            // Check if within focused pane and get pane-relative coords
                            if let Some((pane_x, pane_y)) = wm.screen_to_pane_coords(
                                mouse_event.column,
                                content_y
                            ) {
                                let (sgr, urxvt) = wm.focused_pane_mouse_mode();
                                
                                // Create adjusted event with pane-relative coordinates
                                let adjusted_event = crossterm::event::MouseEvent {
                                    kind: mouse_event.kind,
                                    column: pane_x,
                                    row: pane_y,
                                    modifiers: mouse_event.modifiers,
                                };
                                
                                let bytes = KeyMapper::encode_mouse_event(&adjusted_event, sgr, urxvt);
                                if !bytes.is_empty() {
                                    let _ = wm.write(&bytes);
                                }
                                continue;
                            }
                        }
                    }
                    
                    // Normal wtmux mouse handling
                    match mouse_event.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            let focus_changed = wm.handle_mouse_down(mouse_event.column, mouse_event.row);
                            if focus_changed {
                                reset_cursor_shape();
                            }
                            renderer.render(wm)?;
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            wm.handle_mouse_drag(mouse_event.column, mouse_event.row);
                            renderer.render(wm)?;
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            if let Some(text) = wm.handle_mouse_up() {
                                if !text.is_empty() {
                                    let _ = copy_to_clipboard(&text);
                                }
                            }
                            renderer.render(wm)?;
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            if mouse_event.row < wm.tab_bar_height {
                                // Right-click on a tab renames that window
                                if let Some(tab_id) = wm.tab_at_position(mouse_event.column) {
                                    if wm.select_tab(tab_id) {
                                        reset_cursor_shape();
                                        wm.force_full_redraw();
                                    }
                                    ui.mode = UiMode::Rename;
                                    ui.rename_target = RenameTarget::Window;
                                    if let Some(tab) = wm.active_tab() {
                                        ui.rename_buffer = tab.name.clone();
                                    }
                                    ui.render(renderer, wm, &theme_list)?;
                                }
                            } else if let Some((pane_id, x, y)) = wm.handle_right_click(mouse_event.column, mouse_event.row) {
                                if wm.pane_title_at(mouse_event.column, mouse_event.row) == Some(pane_id) {
                                    // Right-click on the title row renames the pane
                                    open_rename_pane(&mut ui, wm);
                                    ui.render(renderer, wm, &theme_list)?;
                                } else {
                                    // Show context menu
                                    ui.context_menu.show(pane_id, x, y, wm.width, wm.height);
                                    ui.mode = UiMode::ContextMenu;
                                    ui.render(renderer, wm, &theme_list)?;
                                }
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            wm.handle_scroll(3);
                            renderer.render(wm)?;
                        }
                        MouseEventKind::ScrollDown => {
                            wm.handle_scroll(-3);
                            renderer.render(wm)?;
                        }
                        _ => {}
                    }
                }

                Event::Resize(cols, rows) => {
                    // Buffer resize events; the actual resize happens after debounce.
                    pending_resize = Some((cols, rows));
                    last_resize_time = std::time::Instant::now();
                }

                Event::Paste(text) => {
                    // Terminal paste lands in the composer while it is open
                    if ui.mode == UiMode::MessageComposer {
                        ui.message_composer.insert_str(&text);
                        ui.render(renderer, wm, &theme_list)?;
                        continue;
                    }
                    if ui.mode != UiMode::Normal {
                        // An overlay was painted over the panes; repaint fully
                        // so no fragments linger after it closes.
                        wm.force_full_redraw();
                    }
                    ui.close_mode();
                    wm.prefix_mode = false;
                    wm.scroll_to_bottom();
                    let _ = wm.paste(&text);
                }

                // Host terminal focus: forward to the focused pane
                // (DECSET 1004)
                Event::FocusGained => {
                    if let Some((tab_id, pane_id)) = wm.focused_pane_id() {
                        wm.notify_pane_focus(tab_id, pane_id, true);
                    }
                }
                Event::FocusLost => {
                    if let Some((tab_id, pane_id)) = wm.focused_pane_id() {
                        wm.notify_pane_focus(tab_id, pane_id, false);
                    }
                }
            }
        }
    }

    Ok(())
}

/// Send the message composer buffer to its target pane and close the
/// composer, reporting the outcome on the status bar.
fn send_composer_message(
    wm: &mut WindowManager,
    ui: &mut WmAppState,
    renderer: &mut crate::ui::WmRenderer,
) {
    let text = ui.message_composer.text();
    let target = ui.message_composer.take_target();
    ui.close_mode();
    pop_composer_key_reporting();
    wm.force_full_redraw();
    let Some(target) = target else { return };
    if text.trim().is_empty() {
        renderer.set_status_message("empty message; nothing sent".to_string());
    } else {
        match wm.send_message_to_pane(target.tab_id, target.pane_id, &text) {
            Ok(()) => {
                // Record for Ctrl+P/N recall and drop the pending draft;
                // on failure the draft is kept for the next open instead.
                ui.message_composer.record_sent(&text);
                renderer.set_status_message(format!("sent to {}", target.label));
            }
            Err(e) => renderer.set_status_message(format!("send failed: {e}")),
        }
    }
}

/// While the composer is open, ask the host terminal to report Ctrl+Enter
/// distinctly from Enter (kitty keyboard protocol, disambiguate flag).
/// Terminals without support ignore the sequence; there Ctrl+S is the send
/// key. Windows console input reports Ctrl+Enter natively, so this is
/// unix-only.
fn push_composer_key_reporting() {
    #[cfg(unix)]
    {
        use crossterm::event::{KeyboardEnhancementFlags, PushKeyboardEnhancementFlags};
        let _ = execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
    }
}

/// Undo [`push_composer_key_reporting`] when the composer closes.
fn pop_composer_key_reporting() {
    #[cfg(unix)]
    {
        use crossterm::event::PopKeyboardEnhancementFlags;
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
    }
}

/// Dashboard-style label for the focused pane, shown in the message
/// composer title.
fn focused_pane_label(wm: &WindowManager) -> String {
    wm.agent_overview()
        .iter()
        .find(|entry| entry.is_focused)
        .map(|entry| {
            format!(
                "{}:{} · {}: {}",
                entry.window_number, entry.window_name, entry.pane_number, entry.pane_title
            )
        })
        .unwrap_or_else(|| "focused pane".to_string())
}

/// Geometry of the display-popup pane: centered, 60% of the terminal,
/// clamped to sensible minimums.
fn popup_geometry(cols: u16, rows: u16) -> (u16, u16, u16, u16) {
    let min_w = 24.min(cols.saturating_sub(2).max(1));
    let min_h = 8.min(rows.saturating_sub(2).max(1));
    let w = ((cols as u32 * 3 / 5) as u16).max(min_w).min(cols);
    let h = ((rows as u32 * 3 / 5) as u16).max(min_h).min(rows);
    let x = cols.saturating_sub(w) / 2;
    let y = rows.saturating_sub(h) / 2;
    (x, y, w, h)
}

/// Open a display-popup: a floating pane running `command` (or the default
/// shell). With `hold` the popup stays open after the process exits (any
/// key closes it); otherwise it closes on exit. `Prefix, x` force-closes.
fn open_popup(
    wm: &mut WindowManager,
    ui: &mut WmAppState,
    command: Option<&str>,
    hold: bool,
) -> Result<(), String> {
    ui.close_mode(); // drop any existing popup or other modal state

    let (x, y, w, h) = popup_geometry(wm.width, wm.height);
    let mut pane = crate::wm::Pane::new(POPUP_PANE_ID, w, h);
    pane.move_to(x, y);
    pane.focused = true;
    pane.title = Some(match command {
        Some(cmd) => format!("popup: {cmd}"),
        None => "popup".to_string(),
    });

    // Popups are not addressable window panes; keep children from inheriting
    // the previously spawned pane's id.
    env::set_var("WTMUX_PANE", "0.0");
    let shell = command
        .map(str::to_string)
        .or_else(|| wm.default_shell.clone());
    pane.session
        .start_with_options(shell.as_deref(), wm.default_codepage, false)
        .map_err(|e| e.to_string())?;

    ui.popup = Some(pane);
    ui.popup_hold = hold;
    ui.mode = UiMode::Popup;
    Ok(())
}

/// Fixed pane id for the popup (never collides with tab pane numbering,
/// which is only unique per tab anyway).
const POPUP_PANE_ID: u64 = 0;

/// Execute a parsed command-prompt action and return the status message.
/// `display-popup` is handled by the caller (it needs UI state).
fn execute_prompt_action(
    wm: &mut WindowManager,
    action: crate::command_prompt::PromptAction,
) -> String {
    use crate::command_prompt::PromptAction as P;

    match action {
        P::Split { direction } => {
            match direction {
                SplitDirection::Horizontal => wm.split_horizontal(),
                SplitDirection::Vertical => wm.split_vertical(),
            };
            "pane split".to_string()
        }
        P::NewWindow => {
            wm.new_tab();
            "window created".to_string()
        }
        P::KillPane => {
            wm.close_pane();
            "pane killed".to_string()
        }
        P::KillWindow => {
            wm.close_tab();
            "window killed".to_string()
        }
        P::NextWindow => {
            wm.next_tab();
            "next window".to_string()
        }
        P::PrevWindow => {
            wm.prev_tab();
            "previous window".to_string()
        }
        P::LastWindow => {
            wm.last_tab();
            "last window".to_string()
        }
        P::SelectWindow(n) => {
            wm.goto_tab(n);
            format!("window {n}")
        }
        P::RenameWindow(name) => {
            wm.rename_active_tab(&name);
            format!("window renamed to {name:?}")
        }
        P::RenamePane(name) => {
            wm.rename_focused_pane(&name);
            if name.is_empty() {
                "pane title reset".to_string()
            } else {
                format!("pane renamed to {name:?}")
            }
        }
        P::SelectLayout(layout) => {
            wm.set_layout_preset(layout);
            format!("layout: {layout:?}")
        }
        P::ToggleZoom => {
            wm.toggle_zoom();
            "zoom toggled".to_string()
        }
        P::SetSyncPanes(value) => {
            let state = match value {
                None => wm.toggle_broadcast(),
                Some(v) => wm.set_broadcast(v),
            };
            format!("synchronize-panes: {}", if state { "on" } else { "off" })
        }
        P::PipePane => match wm.toggle_pipe_log() {
            Some((true, path)) => format!("logging → {}", path.display()),
            Some((false, _)) => "logging stopped".to_string(),
            None => "pipe-pane: could not start logging".to_string(),
        },
        // Handled by the caller; kept for exhaustiveness
        P::DisplayPopup { .. } => String::new(),
    }
}

/// Dispatch `[hooks]` commands for a batch of agent state transitions.
fn run_agent_state_hooks(hooks: &crate::config::HooksConfig, events: &[crate::wm::AgentStateEvent]) {
    use crate::wm::AgentState;

    for event in events {
        let command = match event.to {
            AgentState::Working => &hooks.on_agent_working,
            AgentState::Blocked => &hooks.on_agent_blocked,
            AgentState::Done => &hooks.on_agent_done,
            AgentState::Idle => &hooks.on_agent_idle,
        };
        if command.is_empty() {
            continue;
        }
        spawn_hook_command(command, event);
    }
}

/// Run a hook command detached, with the transition context in env vars.
/// Hook failures are logged but never disturb the terminal session.
fn spawn_hook_command(command_line: &str, event: &crate::wm::AgentStateEvent) {
    use std::process::{Command, Stdio};

    let mut command = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command_line]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command_line]);
        c
    };
    command
        .env("WTMUX_HOOK_STATE", event.to.label())
        .env("WTMUX_HOOK_PREV_STATE", event.from.label())
        .env("WTMUX_HOOK_PANE", format!("{}.{}", event.tab_id, event.pane_id))
        .env("WTMUX_HOOK_WINDOW", &event.window_name)
        .env("WTMUX_HOOK_TITLE", &event.pane_title)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    match command.spawn() {
        Ok(mut child) => {
            // Reap the child off-thread so it never blocks the event loop
            // (and never lingers as a zombie on Unix).
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(e) => {
            info!("agent hook failed to spawn ({command_line:?}): {e}");
        }
    }
}

/// Open the rename popup for the focused pane, prefilled with its custom
/// title (empty when the pane still shows the default "Pane N" title).
fn open_rename_pane(ui: &mut WmAppState, wm: &WindowManager) {
    ui.mode = UiMode::Rename;
    ui.rename_target = RenameTarget::Pane;
    ui.rename_buffer = wm.focused_pane_title().unwrap_or_default();
}

/// Translate a key binding into the `AppAction` that carries it out.
///
/// Returns `None` for bindings that open a modal UI (see [`apply_ui_action`])
/// and for ones with nothing to do.
fn app_action_for(bound: BoundAction, wm: &WindowManager) -> Option<AppAction> {
    use BoundAction as B;
    let action = match bound {
        B::Noop | B::Detach => return None,
        B::NewWindow => AppAction::NewTab,
        B::KillPane => AppAction::ClosePane,
        B::KillWindow => AppAction::CloseTab,
        B::SplitVertical => AppAction::SplitVertical,
        B::SplitHorizontal => AppAction::SplitHorizontal,
        B::NextWindow => AppAction::NextTab,
        B::PrevWindow => AppAction::PrevTab,
        B::LastWindow => AppAction::LastTab,
        B::SelectWindow(index) => AppAction::GotoTab(index),
        B::SelectPaneDir { direction, forward } => AppAction::FocusDirection { direction, forward },
        B::ResizePaneDir {
            direction,
            arrow_up_or_left,
        } => AppAction::ResizePaneDirection {
            direction,
            arrow_up_or_left,
        },
        B::NextPane => AppAction::FocusNextPane,
        B::PrevPane => AppAction::FocusPrevPane,
        B::ToggleZoom => AppAction::ToggleZoom,
        B::NextLayout => AppAction::NextLayout,
        B::SelectLayout(layout) => AppAction::SelectLayout(layout),
        B::ResizePane { grow } => AppAction::ResizePane { grow },
        B::SwapPaneNext => AppAction::SwapPaneNext,
        B::SwapPanePrev => AppAction::SwapPanePrev,
        B::ToggleBroadcast => AppAction::ToggleBroadcast,
        B::TogglePipeLog => AppAction::TogglePipeLog,
        B::NextAttention => AppAction::FocusNextAttention,
        B::ResetCursorShape => AppAction::ResetCursorShape,
        B::Paste => AppAction::PasteFromClipboard,
        B::SendPrefix => {
            let ch = wm.prefix_key.char;
            AppAction::SendPrefixToPane {
                byte: (ch as u8).wrapping_sub(b'a').wrapping_add(1),
            }
        }
        B::ScrollUp(lines) => AppAction::ScrollUp(lines),
        B::ScrollDown(lines) => AppAction::ScrollDown(lines),
        B::ScrollTop => AppAction::ScrollTop,
        B::ScrollBottom => AppAction::ScrollBottom,
        B::ExtendSelection {
            direction,
            arrow_up_or_left,
        } => AppAction::ExtendSelection(match (direction, arrow_up_or_left) {
            (SplitDirection::Horizontal, true) => KeyCode::Left,
            (SplitDirection::Horizontal, false) => KeyCode::Right,
            (SplitDirection::Vertical, true) => KeyCode::Up,
            (SplitDirection::Vertical, false) => KeyCode::Down,
        }),
        B::CopySelection => AppAction::CopySelection,
        // The `Ui*` variants; handled by `apply_ui_action`.
        _ => return None,
    };
    Some(action)
}

/// Run a binding that opens a modal UI, returning whether it did so.
///
/// These cannot go through `apply_app_action` because they mutate the event
/// loop's own `WmAppState`; the caller re-renders through `ui.render` instead
/// of `renderer.render`.
fn apply_ui_action(
    bound: BoundAction,
    ui: &mut WmAppState,
    wm: &mut WindowManager,
    renderer: &mut crate::ui::WmRenderer,
) -> bool {
    use BoundAction as B;
    if !bound.is_ui() {
        return false;
    }

    ui.close_mode();
    match bound {
        B::UiRenameWindow => {
            ui.mode = UiMode::Rename;
            ui.rename_target = RenameTarget::Window;
            if let Some(tab) = wm.active_tab() {
                ui.rename_buffer = tab.name.clone();
            }
        }
        B::UiRenamePane => {
            open_rename_pane(ui, wm);
        }
        B::UiCopyMode => {
            ui.copy_mode.enter(wm);
            ui.mode = UiMode::CopyMode;
        }
        B::UiSearch => {
            ui.copy_mode.enter(wm);
            ui.copy_mode.enter_search(true);
            ui.mode = UiMode::CopyMode;
        }
        B::UiThemeSelector => {
            ui.mode = UiMode::ThemeSelector;
            ui.theme_selector_index = 0;
        }
        B::UiWindowSelector => {
            ui.window_selector.open(wm);
            ui.mode = UiMode::WindowSelector;
        }
        B::UiCommandPrompt => {
            ui.mode = UiMode::CommandPrompt;
        }
        B::UiAgentDashboard => {
            ui.agent_dashboard.open(wm);
            ui.mode = UiMode::AgentDashboard;
        }
        B::UiMessageComposer => match wm.resolve_target_pane(None) {
            Ok((tab_id, pane_id)) => {
                let label = focused_pane_label(wm);
                ui.message_composer.open(tab_id, pane_id, label);
                ui.mode = UiMode::MessageComposer;
                push_composer_key_reporting();
            }
            Err(e) => renderer.set_status_message(e),
        },
        B::UiDisplayPanes => {
            ui.mode = UiMode::PaneNumbers;
            ui.pane_numbers_started = std::time::Instant::now();
        }
        B::UiHistorySelector => {
            let selector = ui.history_selector_mut();
            selector.show();
            ui.mode = UiMode::HistorySelector;
        }
        // `is_ui()` above admits exactly the arms handled here.
        _ => unreachable!("non-UI action reached apply_ui_action: {bound:?}"),
    }

    true
}

fn apply_app_action(wm: &mut WindowManager, action: AppAction) {
    match action {
        AppAction::Noop => {}
        AppAction::NewTab => {
            wm.new_tab();
        }
        AppAction::ClosePane => {
            wm.close_pane();
        }
        AppAction::CloseTab => {
            wm.close_tab();
        }
        AppAction::SplitHorizontal => {
            wm.split_horizontal();
        }
        AppAction::SplitVertical => {
            wm.split_vertical();
        }
        AppAction::NextTab => {
            wm.next_tab();
        }
        AppAction::PrevTab => {
            wm.prev_tab();
        }
        AppAction::LastTab => {
            wm.last_tab();
        }
        AppAction::FocusDirection { direction, forward } => {
            wm.focus_direction(direction, forward);
            reset_cursor_shape();
        }
        AppAction::ResizePaneDirection {
            direction,
            arrow_up_or_left,
        } => {
            wm.resize_pane_direction(direction, arrow_up_or_left);
        }
        AppAction::GotoTab(num) => {
            wm.goto_tab(num);
        }
        AppAction::FocusNextPane => {
            wm.focus_next_pane();
            reset_cursor_shape();
        }
        AppAction::FocusPrevPane => {
            wm.focus_prev_pane();
            reset_cursor_shape();
        }
        AppAction::ResetCursorShape => {
            reset_cursor_shape();
        }
        AppAction::ToggleZoom => {
            wm.toggle_zoom();
        }
        AppAction::NextLayout => {
            wm.next_layout();
        }
        AppAction::SelectLayout(layout) => {
            wm.set_layout_preset(layout);
        }
        AppAction::ResizePane { grow } => {
            wm.resize_pane(grow);
        }
        AppAction::SwapPaneNext => {
            wm.swap_pane_next();
        }
        AppAction::SwapPanePrev => {
            wm.swap_pane_prev();
        }
        AppAction::PasteFromClipboard => {
            let _ = wm.paste_from_clipboard();
        }
        AppAction::SendPrefixToPane { byte } => {
            let _ = wm.write(&[byte]);
        }
        AppAction::ToggleBroadcast => {
            let state = wm.toggle_broadcast();
            info!("input broadcast toggled: {}", state);
        }
        AppAction::FocusNextAttention => {
            if wm.focus_next_attention() {
                wm.force_full_redraw();
                reset_cursor_shape();
            }
        }
        AppAction::TogglePipeLog => match wm.toggle_pipe_log() {
            Some((enabled, path)) => {
                info!("pipe log {}: {:?}", if enabled { "started" } else { "stopped" }, path);
            }
            None => {
                info!("pipe log toggle failed");
            }
        },
        AppAction::ScrollUp(lines) => {
            wm.handle_scroll(lines.min(i16::MAX as usize) as i16);
        }
        AppAction::ScrollDown(lines) => {
            wm.handle_scroll(-(lines.min(i16::MAX as usize) as i16));
        }
        AppAction::ScrollTop => {
            wm.scroll_to_top();
        }
        AppAction::ScrollBottom => {
            wm.scroll_to_bottom();
        }
        AppAction::ExtendSelection(direction) => {
            if let Some(state) = wm.focused_state_mut() {
                handle_selection_key(state, direction);
                state.active_screen_mut().mark_all_dirty();
            }
        }
        AppAction::CopySelection => {
            if let Some(text) = wm
                .focused_state_mut()
                .and_then(|state| state.get_selected_text())
            {
                if !text.is_empty() {
                    let _ = copy_to_clipboard(&text);
                }
            }
        }
    }
}

/// Main event loop
fn run_main_loop(
    session: &mut Session,
    renderer: &mut Renderer,
    keybindings: ParsedKeyBindings,
) -> anyhow::Result<()> {
    // Adaptive polling: see the wm event loop for rationale
    let active_poll = Duration::from_millis(10);
    let idle_poll = Duration::from_millis(50);
    let mut idle_ticks: u32 = 0;
    let mut status_publisher = tmux_compat::StatusPublisher::default();

    loop {
        // Check if session is still running at the start of each iteration
        if !session.is_running() {
            info!("Session ended");
            break;
        }

        // Process PTY output
        match session.process_output() {
            Ok(true) => {
                // Output processed, render
                idle_ticks = 0;
                renderer.render(&session.state)?;
                session.state.active_screen_mut().clear_dirty();
            }
            Ok(false) => {
                // No output, check again
                idle_ticks = idle_ticks.saturating_add(1);
                if !session.is_running() {
                    info!("Session ended (no output)");
                    break;
                }
            }
            Err(e) => {
                // Read error
                if !session.is_running() {
                    info!("Session ended with error: {}", e);
                    break;
                }
            }
        }

        status_publisher.publish(&tmux_compat::PaneSnapshot::from_session(session));

        // OSC 52: the child asked to set the host clipboard
        if let Some(text) = session.state.osc52.take() {
            let _ = copy_to_clipboard(&text);
        }

        // Process input events
        let poll_timeout = if idle_ticks > 50 { idle_poll } else { active_poll };
        if input::poll(poll_timeout)? {
            idle_ticks = 0;
            let evt = input::read()?;
            // Log all events to debug file
            renderer.log_mouse_event(&format!("Event received: {:?}", evt));
            
            match evt {
                Event::Key(key_event) => {
                    // Only process key press events; key releases are
                    // forwarded when the pane enabled kitty event reporting
                    if key_event.kind != KeyEventKind::Press {
                        if key_event.kind == crossterm::event::KeyEventKind::Release {
                            if let Some(bytes) =
                                KeyMapper::map_for_pane(&key_event, &session.state)
                            {
                                let _ = session.write(&bytes);
                            }
                        }
                        continue;
                    }

                    if keybindings.scrollback_up.matches(&key_event) {
                        let screen = session.state.active_screen_mut();
                        screen.scroll_view_up(10);
                        renderer.render(&session.state)?;
                        continue;
                    }
                    if keybindings.scrollback_down.matches(&key_event) {
                        let screen = session.state.active_screen_mut();
                        screen.scroll_view_down(10);
                        renderer.render(&session.state)?;
                        continue;
                    }
                    if keybindings.scrollback_top.matches(&key_event) {
                        // Scroll to top of history
                        let screen = session.state.active_screen_mut();
                        let max = screen.scrollback.len();
                        screen.scroll_offset = max;
                        screen.mark_all_dirty();
                        renderer.render(&session.state)?;
                        continue;
                    }
                    if keybindings.scrollback_bottom.matches(&key_event) {
                        // Scroll to bottom (live)
                        let screen = session.state.active_screen_mut();
                        screen.scroll_to_bottom();
                        renderer.render(&session.state)?;
                        continue;
                    }

                    let selection_direction = if keybindings.selection_left.matches(&key_event) {
                        Some(KeyCode::Left)
                    } else if keybindings.selection_right.matches(&key_event) {
                        Some(KeyCode::Right)
                    } else if keybindings.selection_up.matches(&key_event) {
                        Some(KeyCode::Up)
                    } else if keybindings.selection_down.matches(&key_event) {
                        Some(KeyCode::Down)
                    } else {
                        None
                    };

                    if let Some(direction) = selection_direction {
                        handle_selection_key(&mut session.state, direction);
                        session.state.active_screen_mut().full_redraw = true;
                        renderer.render(&session.state)?;
                        session.state.active_screen_mut().clear_dirty();
                        continue;
                    }

                    if keybindings.copy_selection.matches(&key_event) {
                        if let Some(text) = session.state.get_selected_text() {
                            if !text.is_empty() {
                                let _ = copy_to_clipboard(&text);
                            }
                        }
                        continue;
                    }

                    // Escape to clear selection
                    if key_event.code == KeyCode::Esc && session.state.selection.is_some() {
                        session.state.clear_selection();
                        session.state.active_screen_mut().full_redraw = true;
                        renderer.render(&session.state)?;
                        session.state.active_screen_mut().clear_dirty();
                        continue;
                    }

                    // Any other key input returns to live view and clears selection
                    {
                        let screen = session.state.active_screen_mut();
                        if screen.is_scrolled() {
                            screen.scroll_to_bottom();
                        }
                    }
                    // Clear selection on regular typing
                    if session.state.selection.is_some() {
                        session.state.clear_selection();
                    }

                    // Map key to bytes and send to PTY
                    if let Some(bytes) = KeyMapper::map_for_pane(&key_event, &session.state) {
                        if let Err(e) = session.write(&bytes) {
                            error!("Failed to write to PTY: {}", e);
                        }
                    }
                }

                Event::Resize(cols, rows) => {
                    info!("Resize: {}x{}", cols, rows);
                    if let Err(e) = session.resize(cols, rows) {
                        error!("Failed to resize: {}", e);
                    }
                    // Force full redraw after resize
                    session.state.active_screen_mut().full_redraw = true;
                    renderer.render(&session.state)?;
                    session.state.active_screen_mut().clear_dirty();
                }

                Event::Paste(text) => {
                    session.state.active_screen_mut().scroll_to_bottom();

                    // Normalise all line endings to CR only (one Enter per newline).
                    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
                    let bytes = if session.state.modes.bracketed_paste {
                        format!("\x1b[200~{}\x1b[201~", normalized).into_bytes()
                    } else {
                        normalized.into_bytes()
                    };
                    if let Err(e) = session.write(&bytes) {
                        error!("Failed to paste: {}", e);
                    }
                }

                Event::Mouse(mouse_event) => {
                    use crossterm::event::{MouseEventKind, MouseButton};
                    
                    // Debug: log mouse event
                    renderer.log_mouse_event(&format!("Mouse event: {:?}", mouse_event));
                    
                    // Check for mouse passthrough to child application
                    // Shift key bypasses passthrough for text selection
                    let shift_held = mouse_event.modifiers.contains(KeyModifiers::SHIFT);
                    
                    if !shift_held && session.state.modes.mouse_enabled() {
                        // Child app has mouse tracking enabled, pass through the event
                        let (sgr, urxvt) = (
                            session.state.modes.mouse_sgr_mode,
                            session.state.modes.mouse_urxvt_mode,
                        );
                        
                        let bytes = KeyMapper::encode_mouse_event(&mouse_event, sgr, urxvt);
                        if !bytes.is_empty() {
                            let _ = session.write(&bytes);
                        }
                        continue;
                    }
                    
                    // Normal simple mode mouse handling
                    match mouse_event.kind {
                        MouseEventKind::Down(MouseButton::Left) => {
                            renderer.log_mouse_event(&format!("Left down at ({}, {})", mouse_event.column, mouse_event.row));
                            // Start selection
                            session.state.start_selection(mouse_event.column, mouse_event.row);
                            session.state.active_screen_mut().full_redraw = true;
                            renderer.render(&session.state)?;
                            session.state.active_screen_mut().clear_dirty();
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            renderer.log_mouse_event(&format!("Left drag at ({}, {})", mouse_event.column, mouse_event.row));
                            // Update selection - only affected rows are marked dirty
                            session.state.update_selection(mouse_event.column, mouse_event.row);
                            renderer.render(&session.state)?;
                            session.state.active_screen_mut().clear_dirty();
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            renderer.log_mouse_event("Left up - copying to clipboard");
                            // End selection and copy to clipboard
                            session.state.end_selection();
                            if let Some(text) = session.state.get_selected_text() {
                                renderer.log_mouse_event(&format!("Selected text: {:?}", text));
                                if !text.is_empty() {
                                    // Copy to clipboard using OSC 52
                                    let b64 = base64_encode(&text);
                                    let osc52 = format!("\x1b]52;c;{}\x07", b64);
                                    print!("{}", osc52);
                                    let _ = std::io::stdout().flush();

                                    // Also try the system clipboard
                                    let _ = copy_to_clipboard(&text);
                                }
                            }
                            // Keep selection visible
                            session.state.active_screen_mut().full_redraw = true;
                            renderer.render(&session.state)?;
                            session.state.active_screen_mut().clear_dirty();
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            // Clear selection on right click
                            session.state.clear_selection();
                            session.state.active_screen_mut().full_redraw = true;
                            renderer.render(&session.state)?;
                            session.state.active_screen_mut().clear_dirty();
                        }
                        MouseEventKind::ScrollUp => {
                            let screen = session.state.active_screen_mut();
                            screen.scroll_view_up(3);
                            renderer.render(&session.state)?;
                            session.state.active_screen_mut().clear_dirty();
                        }
                        MouseEventKind::ScrollDown => {
                            let screen = session.state.active_screen_mut();
                            screen.scroll_view_down(3);
                            renderer.render(&session.state)?;
                            session.state.active_screen_mut().clear_dirty();
                        }
                        _ => {}
                    }
                }

                // Host terminal focus: forward to the pane (DECSET 1004)
                Event::FocusGained => {
                    if session.state.modes.focus_reporting {
                        let _ = session.write(b"\x1b[I");
                    }
                }
                Event::FocusLost => {
                    if session.state.modes.focus_reporting {
                        let _ = session.write(b"\x1b[O");
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle keyboard selection with Shift+Arrow keys
fn handle_selection_key(state: &mut crate::core::term::TerminalState, key: KeyCode) {
    let cursor = state.active_cursor();
    let cols = state.cols;
    let rows = state.rows as usize;
    
    // Convert cursor position to absolute buffer row
    let cursor_abs_row = state.active_screen().screen_to_buffer_row(cursor.row as usize);
    
    // Get current cursor position as starting point if no selection
    let (start_col, start_row): (u16, usize) = if let Some(ref sel) = state.selection {
        sel.end
    } else {
        // Start new selection from cursor position
        let pos = (cursor.col, cursor_abs_row);
        state.selection = Some(crate::core::term::Selection {
            start: pos,
            end: pos,
            active: true,
        });
        pos
    };
    
    // Calculate new end position
    let (new_col, new_row): (u16, usize) = match key {
        KeyCode::Left => {
            if start_col > 0 {
                (start_col - 1, start_row)
            } else if start_row > 0 {
                (cols - 1, start_row - 1)
            } else {
                (start_col, start_row)
            }
        }
        KeyCode::Right => {
            if start_col < cols - 1 {
                (start_col + 1, start_row)
            } else if start_row < rows - 1 {
                (0, start_row + 1)
            } else {
                (start_col, start_row)
            }
        }
        KeyCode::Up => {
            if start_row > 0 {
                (start_col, start_row - 1)
            } else {
                (start_col, start_row)
            }
        }
        KeyCode::Down => {
            if start_row < rows - 1 {
                (start_col, start_row + 1)
            } else {
                (start_col, start_row)
            }
        }
        _ => (start_col, start_row),
    };
    
    // Update selection end
    if let Some(ref mut sel) = state.selection {
        sel.end = (new_col, new_row);
    }
}

/// Simple base64 encoding
fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    
    let bytes = input.as_bytes();
    let mut result = String::new();
    
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).map(|&b| b as u32).unwrap_or(0);
        let b2 = chunk.get(2).map(|&b| b as u32).unwrap_or(0);
        
        let n = (b0 << 16) | (b1 << 8) | b2;
        
        result.push(ALPHABET[(n >> 18) as usize & 0x3F] as char);
        result.push(ALPHABET[(n >> 12) as usize & 0x3F] as char);
        
        if chunk.len() > 1 {
            result.push(ALPHABET[(n >> 6) as usize & 0x3F] as char);
        } else {
            result.push('=');
        }
        
        if chunk.len() > 2 {
            result.push(ALPHABET[n as usize & 0x3F] as char);
        } else {
            result.push('=');
        }
    }
    
    result
}

/// Copy text to the system clipboard
#[cfg(windows)]
fn copy_to_clipboard(text: &str) -> Result<(), ()> {
    copy_to_clipboard_windows(text)
}

/// Read text from the system clipboard, if any.
fn read_clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .and_then(|mut clipboard| clipboard.get_text())
        .ok()
        .filter(|text| !text.is_empty())
}

/// Returns a process-wide clipboard handle, reused across calls.
///
/// On X11/XWayland, clipboard content is only served for as long as the
/// owning `arboard::Clipboard` handle stays alive; creating a fresh one per
/// call and dropping it immediately releases selection ownership right
/// away, so copies silently disappear even though `set_text` reports Ok.
#[cfg(not(windows))]
fn clipboard_handle() -> Option<&'static std::sync::Mutex<Option<arboard::Clipboard>>> {
    use std::sync::{Mutex, OnceLock};
    static CLIPBOARD: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    let mutex = CLIPBOARD.get_or_init(|| Mutex::new(arboard::Clipboard::new().ok()));
    {
        let mut guard = mutex.lock().ok()?;
        if guard.is_none() {
            *guard = arboard::Clipboard::new().ok();
        }
    }
    Some(mutex)
}

/// Copy text to the system clipboard
#[cfg(not(windows))]
fn copy_to_clipboard(text: &str) -> Result<(), ()> {
    let mutex = clipboard_handle().ok_or(())?;
    let mut guard = mutex.lock().map_err(|_| ())?;
    let clipboard = guard.as_mut().ok_or(())?;
    clipboard.set_text(text.to_string()).map_err(|_| ())
}

/// Copy text to Windows clipboard
#[cfg(windows)]
fn copy_to_clipboard_windows(text: &str) -> Result<(), ()> {
    use std::ptr;
    use windows::Win32::System::DataExchange::{
        OpenClipboard, CloseClipboard, EmptyClipboard, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::Foundation::{HWND, HANDLE, HGLOBAL};
    
    unsafe {
        if OpenClipboard(HWND::default()).is_err() {
            return Err(());
        }
        
        let _ = EmptyClipboard();
        
        // Convert to UTF-16
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let size = wide.len() * 2;
        
        let hmem = GlobalAlloc(GMEM_MOVEABLE, size).map_err(|_| ())?;
        let hglobal = HGLOBAL(hmem.0);
        let ptr = GlobalLock(hglobal);
        
        if !ptr.is_null() {
            ptr::copy_nonoverlapping(wide.as_ptr(), ptr as *mut u16, wide.len());
            let _ = GlobalUnlock(hglobal);
            
            // CF_UNICODETEXT = 13
            let _ = SetClipboardData(13, HANDLE(hmem.0));
        }
        
        let _ = CloseClipboard();
    }
    
    Ok(())
}

