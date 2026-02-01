//! wtmux - A tmux-like terminal multiplexer for Windows
//!
//! wtmux provides tmux-style window/pane management using ConPTY on Windows.
//! Features include multiple tabs, split panes, and familiar keybindings.
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

use std::env;
use std::io::Write;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::cursor::SetCursorStyle;
use crossterm::execute;
use tracing::{error, info, Level};
use tracing_subscriber::FmtSubscriber;

use crate::core::session::Session;
use crate::ui::{KeyMapper, Renderer, ContextMenu, ContextMenuAction};
use crate::wm::{WindowManager, SplitDirection};
use crate::history::HistorySelector;
use crate::config::{Config as WtmuxConfig, ColorScheme};
use crate::copymode::CopyMode;

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            shell: None,  // Will be set from config.toml or default to cmd.exe
            native_console: false,
            codepage: Some(65001), // UTF-8 by default
            multipane: true, // Multi-pane mode is now default
            shell_from_cli: false,
        }
    }
}

/// Version string from Cargo.toml
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn print_version() {
    eprintln!("wtmux {}", VERSION);
}

fn print_help() {
    eprintln!("wtmux {} - A tmux-like terminal multiplexer for Windows", VERSION);
    eprintln!();
    eprintln!("Usage: wtmux [OPTIONS]");
    eprintln!();
    eprintln!("Mode options:");
    eprintln!("  (default)             Multi-pane mode (tmux-like)");
    eprintln!("  -1, --simple          Simple single-pane mode");
    eprintln!();
    eprintln!("Shell options:");
    eprintln!("  (default)             From config.toml or Command Prompt (cmd.exe)");
    eprintln!("  -c, --cmd             Command Prompt (cmd.exe)");
    eprintln!("  -p, --powershell      Windows PowerShell (powershell.exe)");
    eprintln!("  -7, --pwsh            PowerShell 7 (pwsh.exe)");
    eprintln!("  -w, --wsl             WSL (Windows Subsystem for Linux)");
    eprintln!("  -s, --shell <CMD>     Custom shell command");
    eprintln!();
    eprintln!("Encoding options:");
    eprintln!("  (default)             UTF-8 (CP65001)");
    eprintln!("  --sjis                Shift-JIS mode (CP932)");
    eprintln!();
    eprintln!("Other options:");
    eprintln!("  -n, --native          Run in native console window");
    eprintln!("  -v, --version         Show version");
    eprintln!("  -h, --help            Show this help");
    eprintln!();
    eprintln!("Multi-pane mode keybindings (tmux compatible, Ctrl+B prefix):");
    eprintln!("  Ctrl+B, c             New window (tab)");
    eprintln!("  Ctrl+B, &             Kill window (tab)");
    eprintln!("  Ctrl+B, x             Kill pane");
    eprintln!("  Ctrl+B, \"             Split pane horizontally (top/bottom)");
    eprintln!("  Ctrl+B, %             Split pane vertically (left/right)");
    eprintln!("  Ctrl+B, n             Next window");
    eprintln!("  Ctrl+B, p             Previous window");
    eprintln!("  Ctrl+B, l             Last window (toggle)");
    eprintln!("  Ctrl+B, 0-9           Select window by number");
    eprintln!("  Ctrl+B, o             Next pane");
    eprintln!("  Ctrl+B, ;             Previous pane");
    eprintln!("  Ctrl+B, Arrow         Move to pane in direction");
    eprintln!("  Ctrl+B, z             Toggle pane zoom");
    eprintln!();
    eprintln!("Snippet selector (at command prompt, not in vim/apps):");
    eprintln!("  Ctrl+R                Open snippet selector");
    eprintln!("  ↑/↓                   Navigate snippets");
    eprintln!("  1-9                   Select by number");
    eprintln!("  Enter                 Insert selected snippet");
    eprintln!("  Esc                   Close selector");
    eprintln!("  (type to search)      Filter snippets");
    eprintln!();
    eprintln!("Snippets are stored in: ~/.wtmux/snippets.toml");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  wtmux                 Multi-pane mode (default)");
    eprintln!("  wtmux -7 -u           PowerShell 7, UTF-8");
    eprintln!("  wtmux -w              WSL");
    eprintln!("  wtmux -1              Simple single-pane mode");
    eprintln!();
    eprintln!("Configuration: ~/.wtmux/config.toml");
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
                print_help();
                std::process::exit(0);
            }
            "-v" | "--version" => {
                print_version();
                std::process::exit(0);
            }
            // Mode selection
            "-1" | "--simple" => {
                config.multipane = false;
            }
            // Shell selection
            "-c" | "--cmd" => {
                config.shell = Some("cmd.exe".to_string());
                config.shell_from_cli = true;
            }
            "-p" | "--powershell" => {
                config.shell = Some("powershell.exe".to_string());
                config.shell_from_cli = true;
            }
            "-7" | "--pwsh" => {
                config.shell = Some("pwsh.exe".to_string());
                config.shell_from_cli = true;
            }
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
            arg => {
                return Err(format!("Unknown argument: {}. Use -h for help.", arg));
            }
        }
        i += 1;
    }

    Ok(config)
}

/// Check if running inside Windows Terminal
#[cfg(windows)]
fn is_windows_terminal() -> bool {
    // Check for WT_SESSION environment variable (set by Windows Terminal)
    env::var("WT_SESSION").is_ok()
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
    } else {
        shell_cmd
    }
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
    let wants_native = args.iter().any(|a| a == "-n" || a == "--native");
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

    // Initialize logging to file
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from);
    
    let log_path = home
        .map(|h| h.join(".wtmux").join("wtmux.log"))
        .unwrap_or_else(|| std::path::PathBuf::from("wtmux.log"));
    
    // Create log directory if needed
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    
    // Open log file (append mode)
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();
    
    if let Some(file) = log_file {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(Level::INFO)
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .finish();
        let _ = tracing::subscriber::set_global_default(subscriber);
    }

    info!("wtmux starting...");
    
    // Set environment variable so child processes can detect wtmux
    env::set_var("WTMUX", "1");
    env::set_var("WTMUX_VERSION", env!("CARGO_PKG_VERSION"));

    // Check platform
    #[cfg(not(windows))]
    {
        eprintln!("wtmux currently only supports Windows with ConPTY.");
        eprintln!("Running in demo mode...");
        run_demo()?;
        return Ok(());
    }

    #[cfg(windows)]
    {
        run_terminal(config)?;
    }

    Ok(())
}

/// Run the terminal (Windows only)
#[cfg(windows)]
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
    // Default to cmd.exe if still not set
    if config.shell.is_none() {
        config.shell = Some("cmd.exe".to_string());
    }
    
    // Codepage from config file (CLI always overrides since it has default)
    // Note: codepage is always Some due to default, so we check wtmux_config
    if let Some(cp) = wtmux_config.codepage {
        // Only override if CLI didn't explicitly set a different value
        // For now, config file codepage is not applied (CLI default takes precedence)
        let _ = cp; // Suppress unused warning
    }
    
    // Detect terminal environment
    let terminal_env = detect_terminal_env();
    let shell_cmd_str = config.shell.clone().unwrap_or_else(|| "cmd.exe".to_string());
    let shell_name = get_shell_name(&shell_cmd_str);
    let encoding_name = get_encoding_name(config.codepage);
    
    // Log environment info
    info!("Host terminal: {}", terminal_env);
    info!("Shell: {} ({})", shell_name, shell_cmd_str);
    info!("Encoding: {}", encoding_name);
    info!("Multi-pane mode: {}", config.multipane);
    
    // Get terminal size
    let (cols, rows) = Renderer::size()?;
    info!("Terminal size: {}x{}", cols, rows);

    if config.multipane {
        // Multi-pane mode
        return run_terminal_wm(config, cols, rows, shell_name, encoding_name, &terminal_env, wtmux_config);
    }

    // Simple single-pane mode
    // Create session (ConPTY always outputs UTF-8)
    let mut session = Session::new(1, cols, rows);

    // Start shell with optional codepage
    if let Err(e) = session.start_with_codepage(Some(&shell_cmd_str), config.codepage) {
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
    let result = run_main_loop(&mut session, &mut renderer);

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
#[cfg(windows)]
fn run_terminal_wm(config: Config, cols: u16, rows: u16, shell_name: &str, encoding_name: &str, terminal_env: &str, wtmux_config: WtmuxConfig) -> anyhow::Result<()> {
    use crossterm::terminal;
    use crate::ui::WmRenderer;
    
    // Get color scheme from config
    let color_scheme = wtmux_config.get_color_scheme();
    
    // Parse prefix key from config
    let prefix_key = crate::config::PrefixKey::parse(&wtmux_config.prefix_key)
        .unwrap_or(crate::config::PrefixKey { char: 'b' });
    
    // Create window manager
    let mut wm = WindowManager::new(
        cols, 
        rows, 
        config.shell.clone(),
        config.codepage,
        prefix_key,
    );
    
    // Start initial session
    if let Err(e) = wm.start() {
        error!("Failed to start session: {}", e);
        return Err(anyhow::anyhow!(e));
    }
    
    // Force resize to ensure PTY has correct size
    wm.resize(cols, rows);

    // Initialize renderer with color scheme
    let mut renderer = WmRenderer::with_color_scheme(color_scheme);
    renderer.init()?;
    
    // Set window title
    let title = format!("wtmux [Multi] - {} | {} | {}", shell_name, encoding_name, terminal_env);
    print!("\x1b]0;{}\x07", title);
    let _ = std::io::stdout().flush();

    // Run main loop
    let result = run_wm_main_loop(&mut wm, &mut renderer);

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
#[cfg(windows)]
fn run_wm_main_loop(wm: &mut WindowManager, renderer: &mut crate::ui::WmRenderer) -> anyhow::Result<()> {
    let poll_timeout = Duration::from_millis(10);
    let mut selector = HistorySelector::new();
    
    // Theme selector state
    let mut theme_selector_visible = false;
    let mut theme_selector_index: usize = 0;
    let theme_list = ColorScheme::list();
    
    // Pane numbers display state
    let mut pane_numbers_visible = false;
    let mut pane_numbers_timer = std::time::Instant::now();
    let pane_numbers_duration = Duration::from_secs(2);
    
    // Copy mode state
    let mut copy_mode = CopyMode::new();
    
    // Window rename mode state
    let mut rename_mode = false;
    let mut rename_buffer = String::new();
    
    // Context menu state
    let mut context_menu = ContextMenu::new();

    loop {
        // Check if any session is still running
        if !wm.is_running() {
            info!("All sessions ended");
            break;
        }
        
        // Check pane numbers timeout
        if pane_numbers_visible && pane_numbers_timer.elapsed() >= pane_numbers_duration {
            pane_numbers_visible = false;
        }

        // Process output from all panes
        let has_output = wm.process_output();
        
        // Check again after processing output (panes may have exited)
        if !wm.is_running() {
            info!("All sessions ended after output processing");
            break;
        }
        
        // Render based on current mode
        if copy_mode.active || rename_mode || context_menu.visible {
            // In copy mode, rename mode, or context menu, only render on key events
            // (rendering happens in the key handler below)
        } else if has_output {
            if theme_selector_visible {
                renderer.render_with_theme_selector(wm, &theme_list, theme_selector_index)?;
            } else if pane_numbers_visible {
                renderer.render_with_pane_numbers(wm)?;
            } else {
                renderer.render_with_selector(wm, Some(&selector))?;
            }
        }

        // Poll for events
        if event::poll(poll_timeout)? {
            match event::read()? {
                Event::Key(key_event) => {
                    if key_event.kind != KeyEventKind::Press {
                        continue;
                    }
                    
                    // Handle context menu keyboard navigation
                    if context_menu.visible {
                        match key_event.code {
                            KeyCode::Esc => {
                                context_menu.hide();
                                renderer.render(wm)?;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                context_menu.up();
                                renderer.render_with_context_menu(wm, &context_menu)?;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                context_menu.down();
                                renderer.render_with_context_menu(wm, &context_menu)?;
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                let action = context_menu.selected_action();
                                execute_context_menu_action(wm, action);
                                context_menu.hide();
                                renderer.render(wm)?;
                            }
                            _ => {}
                        }
                        continue;
                    }
                    
                    // Handle copy mode
                    if copy_mode.active {
                        let mut needs_full_redraw = false;
                        let old_scroll = copy_mode.scroll_offset;
                        
                        if copy_mode.search_mode {
                            // Search input mode
                            needs_full_redraw = true;
                            match key_event.code {
                                KeyCode::Esc => {
                                    copy_mode.cancel_search();
                                }
                                KeyCode::Enter => {
                                    copy_mode.execute_search(wm);
                                }
                                KeyCode::Backspace => {
                                    copy_mode.search_backspace();
                                }
                                KeyCode::Char(c) => {
                                    copy_mode.search_input(c);
                                }
                                _ => {}
                            }
                        } else {
                            // Normal copy mode
                            match key_event.code {
                                // Exit copy mode
                                KeyCode::Esc | KeyCode::Char('q') => {
                                    copy_mode.exit();
                                    renderer.render(wm)?;
                                    continue;
                                }
                                // Movement - vim style (cursor only update unless scroll changes)
                                KeyCode::Char('h') | KeyCode::Left => {
                                    copy_mode.cursor_left(wm);
                                }
                                KeyCode::Char('j') | KeyCode::Down => {
                                    copy_mode.cursor_down(wm);
                                }
                                KeyCode::Char('k') | KeyCode::Up => {
                                    copy_mode.cursor_up(wm);
                                }
                                KeyCode::Char('l') | KeyCode::Right => {
                                    copy_mode.cursor_right(wm);
                                }
                                // Line navigation
                                KeyCode::Char('0') => {
                                    copy_mode.line_start();
                                }
                                KeyCode::Char('$') => {
                                    copy_mode.line_end(wm);
                                }
                                // Page navigation - needs full redraw
                                KeyCode::PageUp | KeyCode::Char('b') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    copy_mode.page_up(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::PageDown | KeyCode::Char('f') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    copy_mode.page_down(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('u') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    copy_mode.half_page_up(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('d') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                                    copy_mode.half_page_down(wm);
                                    needs_full_redraw = true;
                                }
                                // Go to top/bottom - needs full redraw
                                KeyCode::Char('g') => {
                                    copy_mode.goto_top(wm);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('G') => {
                                    copy_mode.goto_bottom(wm);
                                    needs_full_redraw = true;
                                }
                                // Selection - needs full redraw
                                KeyCode::Char(' ') | KeyCode::Char('v') => {
                                    copy_mode.toggle_selection();
                                    needs_full_redraw = true;
                                }
                                // Copy
                                KeyCode::Enter | KeyCode::Char('y') => {
                                    if let Some(text) = copy_mode.copy_selection(wm) {
                                        // Copy to clipboard
                                        if let Ok(mut clipboard) = arboard::Clipboard::new() {
                                            let _ = clipboard.set_text(text);
                                        }
                                        copy_mode.exit();
                                        renderer.render(wm)?;
                                        continue;
                                    }
                                }
                                // Search - needs full redraw
                                KeyCode::Char('/') => {
                                    copy_mode.enter_search(true);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('?') => {
                                    copy_mode.enter_search(false);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('n') => {
                                    copy_mode.find_next_match(false);
                                    needs_full_redraw = true;
                                }
                                KeyCode::Char('N') => {
                                    copy_mode.find_prev_match();
                                    needs_full_redraw = true;
                                }
                                _ => {}
                            }
                        }
                        
                        // Check if scroll changed
                        if copy_mode.scroll_offset != old_scroll {
                            needs_full_redraw = true;
                        }
                        
                        // Render
                        if needs_full_redraw || copy_mode.selection_start.is_some() {
                            renderer.render_with_copy_mode(wm, &copy_mode)?;
                        } else {
                            renderer.render_copy_mode_cursor_only(wm, &copy_mode)?;
                        }
                        continue;
                    }
                    
                    // Handle rename mode
                    if rename_mode {
                        match key_event.code {
                            KeyCode::Esc => {
                                rename_mode = false;
                                rename_buffer.clear();
                                renderer.render(wm)?;
                                continue;
                            }
                            KeyCode::Enter => {
                                if !rename_buffer.is_empty() {
                                    wm.rename_active_tab(&rename_buffer);
                                }
                                rename_mode = false;
                                rename_buffer.clear();
                                renderer.render(wm)?;
                                continue;
                            }
                            KeyCode::Backspace => {
                                rename_buffer.pop();
                            }
                            KeyCode::Char(c) => {
                                if rename_buffer.len() < 30 {
                                    rename_buffer.push(c);
                                }
                            }
                            _ => {}
                        }
                        renderer.render_with_rename(wm, &rename_buffer)?;
                        continue;
                    }
                    
                    // Handle pane numbers mode - select pane by number
                    if pane_numbers_visible {
                        if let KeyCode::Char(c) = key_event.code {
                            if c.is_ascii_digit() {
                                let num = c.to_digit(10).unwrap_or(0) as usize;
                                wm.select_pane_by_number(num);
                                reset_cursor_shape();
                            }
                        }
                        pane_numbers_visible = false;
                        renderer.render(wm)?;
                        continue;
                    }

                    // Handle theme selector mode
                    if theme_selector_visible {
                        match key_event.code {
                            KeyCode::Esc => {
                                theme_selector_visible = false;
                            }
                            KeyCode::Up => {
                                if theme_selector_index > 0 {
                                    theme_selector_index -= 1;
                                }
                            }
                            KeyCode::Down => {
                                if theme_selector_index + 1 < theme_list.len() {
                                    theme_selector_index += 1;
                                }
                            }
                            KeyCode::Enter => {
                                let scheme_name = theme_list[theme_selector_index];
                                renderer.set_color_scheme(ColorScheme::by_name(scheme_name));
                                theme_selector_visible = false;
                            }
                            KeyCode::Char(c) if c.is_ascii_digit() => {
                                let num = c.to_digit(10).unwrap_or(0) as usize;
                                if num >= 1 && num <= theme_list.len() {
                                    theme_selector_index = num - 1;
                                    let scheme_name = theme_list[theme_selector_index];
                                    renderer.set_color_scheme(ColorScheme::by_name(scheme_name));
                                    theme_selector_visible = false;
                                }
                            }
                            _ => {}
                        }
                        renderer.render_with_theme_selector(wm, &theme_list, theme_selector_index)?;
                        continue;
                    }

                    // Handle selector mode
                    if selector.visible {
                        match key_event.code {
                            KeyCode::Esc => {
                                selector.hide();
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
                            KeyCode::Char(c) => {
                                // Number selection only when query is empty
                                if selector.query.is_empty() && c.is_ascii_digit() {
                                    if let Some(num) = c.to_digit(10) {
                                        if num >= 1 && num <= 9 {
                                            if let Some(command) = selector.select_number(num as usize) {
                                                // Clear current input and insert
                                                wm.clear_current_input();
                                                let _ = wm.write(command.as_bytes());
                                            }
                                            renderer.render_with_selector(wm, Some(&selector))?;
                                            continue;
                                        }
                                    }
                                }
                                // Add to search query
                                selector.input_char(c);
                            }
                            _ => {}
                        }
                        renderer.render_with_selector(wm, Some(&selector))?;
                        continue;
                    }

                    // Handle prefix mode
                    if wm.prefix_mode {
                        match key_event.code {
                            // Cancel prefix mode (Esc only)
                            KeyCode::Esc => {
                                wm.prefix_mode = false;
                            }
                            // New window (tab)
                            KeyCode::Char('c') => {
                                wm.new_tab();
                                wm.prefix_mode = false;
                            }
                            // Kill pane (tmux: x)
                            KeyCode::Char('x') => {
                                wm.close_pane();
                                wm.prefix_mode = false;
                            }
                            // Kill window/tab (tmux: &)
                            KeyCode::Char('&') => {
                                wm.close_tab();
                                wm.prefix_mode = false;
                            }
                            // Split horizontal (tmux: " splits top/bottom)
                            KeyCode::Char('"') => {
                                wm.split_vertical();
                                wm.prefix_mode = false;
                            }
                            // Split vertical (tmux: % splits left/right)
                            KeyCode::Char('%') => {
                                wm.split_horizontal();
                                wm.prefix_mode = false;
                            }
                            // Next window (tmux: n)
                            KeyCode::Char('n') => {
                                wm.next_tab();
                                wm.prefix_mode = false;
                            }
                            // Previous window (tmux: p)
                            KeyCode::Char('p') => {
                                wm.prev_tab();
                                wm.prefix_mode = false;
                            }
                            // Last window (tmux: l) - toggle between last two tabs
                            KeyCode::Char('l') => {
                                wm.last_tab();
                                wm.prefix_mode = false;
                            }
                            // Move focus between panes (tmux: arrow keys without Ctrl)
                            // Resize panes (tmux: Ctrl+arrow keys)
                            KeyCode::Left => {
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                                    // Left arrow: arrow_up_or_left = true
                                    wm.resize_pane_direction(SplitDirection::Horizontal, true);
                                } else {
                                    wm.focus_direction(SplitDirection::Horizontal, false);
                                    reset_cursor_shape();
                                }
                                wm.prefix_mode = false;
                            }
                            KeyCode::Right => {
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                                    // Right arrow: arrow_up_or_left = false
                                    wm.resize_pane_direction(SplitDirection::Horizontal, false);
                                } else {
                                    wm.focus_direction(SplitDirection::Horizontal, true);
                                    reset_cursor_shape();
                                }
                                wm.prefix_mode = false;
                            }
                            KeyCode::Up => {
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                                    // Up arrow: arrow_up_or_left = true
                                    wm.resize_pane_direction(SplitDirection::Vertical, true);
                                } else {
                                    wm.focus_direction(SplitDirection::Vertical, false);
                                    reset_cursor_shape();
                                }
                                wm.prefix_mode = false;
                            }
                            KeyCode::Down => {
                                if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                                    // Down arrow: arrow_up_or_left = false
                                    wm.resize_pane_direction(SplitDirection::Vertical, false);
                                } else {
                                    wm.focus_direction(SplitDirection::Vertical, true);
                                    reset_cursor_shape();
                                }
                                wm.prefix_mode = false;
                            }
                            // Select window by number (tmux: 0-9) - only when not showing pane numbers
                            KeyCode::Char(c) if c.is_ascii_digit() && !pane_numbers_visible => {
                                let num = c.to_digit(10).unwrap_or(0) as usize;
                                wm.goto_tab(num);
                                wm.prefix_mode = false;
                            }
                            // Next pane (tmux: o)
                            KeyCode::Char('o') => {
                                wm.focus_next_pane();
                                reset_cursor_shape();
                                wm.prefix_mode = false;
                            }
                            // Previous pane (tmux: ;)
                            KeyCode::Char(';') => {
                                wm.focus_prev_pane();
                                reset_cursor_shape();
                                wm.prefix_mode = false;
                            }
                            // Reset cursor shape (tmux: r)
                            KeyCode::Char('r') => {
                                reset_cursor_shape();
                                wm.prefix_mode = false;
                            }
                            // Zoom pane toggle (tmux: z)
                            KeyCode::Char('z') => {
                                wm.toggle_zoom();
                                wm.prefix_mode = false;
                            }
                            // Rename window (tmux: ,)
                            KeyCode::Char(',') => {
                                rename_mode = true;
                                rename_buffer.clear();
                                // Pre-fill with current name
                                if let Some(tab) = wm.active_tab() {
                                    rename_buffer = tab.name.clone();
                                }
                                wm.prefix_mode = false;
                                renderer.render_with_rename(wm, &rename_buffer)?;
                                continue;
                            }
                            // Next layout (tmux: Space)
                            KeyCode::Char(' ') => {
                                wm.next_layout();
                                wm.prefix_mode = false;
                            }
                            // Copy mode (tmux: [)
                            KeyCode::Char('[') => {
                                copy_mode.enter(wm);
                                wm.prefix_mode = false;
                                renderer.render_with_copy_mode(wm, &copy_mode)?;
                                continue;
                            }
                            // Search mode (Ctrl+B, /)
                            KeyCode::Char('/') => {
                                copy_mode.enter(wm);
                                copy_mode.enter_search(true);
                                wm.prefix_mode = false;
                                renderer.render_with_copy_mode(wm, &copy_mode)?;
                                continue;
                            }
                            // Theme/color scheme selector (Ctrl+B, t)
                            KeyCode::Char('t') => {
                                theme_selector_visible = true;
                                theme_selector_index = 0;
                                wm.prefix_mode = false;
                                renderer.render_with_theme_selector(wm, &theme_list, theme_selector_index)?;
                                continue;
                            }
                            // Resize pane (tmux: Ctrl+arrow)
                            KeyCode::Char('+') | KeyCode::Char('=') => {
                                wm.resize_pane(true);
                                wm.prefix_mode = false;
                            }
                            KeyCode::Char('-') => {
                                wm.resize_pane(false);
                                wm.prefix_mode = false;
                            }
                            // Swap pane with next (tmux: })
                            KeyCode::Char('}') => {
                                wm.swap_pane_next();
                                wm.prefix_mode = false;
                            }
                            // Swap pane with previous (tmux: {)
                            KeyCode::Char('{') => {
                                wm.swap_pane_prev();
                                wm.prefix_mode = false;
                            }
                            // Display pane numbers (tmux: q)
                            KeyCode::Char('q') => {
                                pane_numbers_visible = true;
                                pane_numbers_timer = std::time::Instant::now();
                                wm.prefix_mode = false;
                                renderer.render_with_pane_numbers(wm)?;
                                continue;
                            }
                            // Detach (tmux: d) - for now just show message
                            KeyCode::Char('d') => {
                                // Detach not implemented yet
                                wm.prefix_mode = false;
                            }
                            // Send prefix key to application (e.g., Ctrl+B Ctrl+B sends Ctrl+B)
                            KeyCode::Char(c) if c == wm.prefix_key.char => {
                                let ctrl_code = (c as u8) - b'a' + 1;
                                let _ = wm.write(&[ctrl_code]);
                                wm.prefix_mode = false;
                            }
                            _ => {
                                // Unknown command, exit prefix mode
                                wm.prefix_mode = false;
                            }
                        }
                        renderer.render(wm)?;
                        continue;
                    }

                    // Check for prefix key (configurable, default: Ctrl+B)
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) {
                        if key_event.code == KeyCode::Char(wm.prefix_key.char) {
                            wm.prefix_mode = true;
                            renderer.render(wm)?;
                            continue;
                        }
                    }

                    // Check for Ctrl+R (selector) - only when not in alternate screen
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) 
                        && key_event.code == KeyCode::Char('r') 
                        && !wm.is_in_alternate_screen() 
                    {
                        selector.show();
                        renderer.render_with_selector(wm, Some(&selector))?;
                        continue;
                    }

                    // Save command to history on Enter (when not in alternate screen)
                    if key_event.code == KeyCode::Enter && !wm.is_in_alternate_screen() {
                        if let Some(command) = wm.get_current_line() {
                            let stripped = crate::history::strip_prompt(&command);
                            if !stripped.is_empty() {
                                selector.add_to_history(stripped);
                            }
                        }
                    }

                    // Reset scroll to bottom on any key input (return to live view)
                    wm.scroll_to_bottom();

                    // Send key to focused pane
                    let bytes = KeyMapper::map_key(&key_event);
                    if !bytes.is_empty() {
                        let _ = wm.write(&bytes);
                    }
                }

                Event::Mouse(mouse_event) => {
                    use crossterm::event::{MouseEventKind, MouseButton};
                    
                    // Close snippet selector on mouse click outside
                    if selector.visible {
                        selector.hide();
                        renderer.render_with_selector(wm, Some(&selector))?;
                    }
                    
                    // Handle context menu interactions
                    if context_menu.visible {
                        match mouse_event.kind {
                            MouseEventKind::Down(MouseButton::Left) => {
                                if let Some(action) = context_menu.handle_click(mouse_event.column, mouse_event.row) {
                                    // Execute the action
                                    execute_context_menu_action(wm, action);
                                    context_menu.hide();
                                    renderer.render(wm)?;
                                } else {
                                    // Clicked outside menu - close it
                                    context_menu.hide();
                                    renderer.render(wm)?;
                                }
                            }
                            MouseEventKind::Down(MouseButton::Right) => {
                                // Close menu on right click
                                context_menu.hide();
                                renderer.render(wm)?;
                            }
                            MouseEventKind::Moved | MouseEventKind::Drag(_) => {
                                // Highlight item under cursor
                                if context_menu.update_hover(mouse_event.column, mouse_event.row) {
                                    renderer.render_context_menu_only(&context_menu)?;
                                }
                            }
                            _ => {}
                        }
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
                            renderer.render_with_selector(wm, Some(&selector))?;
                        }
                        MouseEventKind::Drag(MouseButton::Left) => {
                            wm.handle_mouse_drag(mouse_event.column, mouse_event.row);
                            renderer.render_with_selector(wm, Some(&selector))?;
                        }
                        MouseEventKind::Up(MouseButton::Left) => {
                            if let Some(text) = wm.handle_mouse_up() {
                                if !text.is_empty() {
                                    // Copy to clipboard
                                    #[cfg(windows)]
                                    {
                                        let _ = copy_to_clipboard_windows(&text);
                                    }
                                }
                            }
                            renderer.render_with_selector(wm, Some(&selector))?;
                        }
                        MouseEventKind::Down(MouseButton::Right) => {
                            // Show context menu
                            if let Some((pane_id, x, y)) = wm.handle_right_click(mouse_event.column, mouse_event.row) {
                                context_menu.show(pane_id, x, y, wm.width, wm.height);
                                renderer.render_with_context_menu(wm, &context_menu)?;
                            }
                        }
                        MouseEventKind::ScrollUp => {
                            wm.handle_scroll(3);
                            renderer.render_with_selector(wm, Some(&selector))?;
                        }
                        MouseEventKind::ScrollDown => {
                            wm.handle_scroll(-3);
                            renderer.render_with_selector(wm, Some(&selector))?;
                        }
                        _ => {}
                    }
                }

                Event::Resize(cols, rows) => {
                    wm.resize(cols, rows);
                    renderer.render_with_selector(wm, Some(&selector))?;
                }

                _ => {}
            }
        }
    }

    Ok(())
}

/// Execute a context menu action
fn execute_context_menu_action(wm: &mut WindowManager, action: ContextMenuAction) {
    match action {
        ContextMenuAction::Paste => {
            let _ = wm.paste_from_clipboard();
        }
        ContextMenuAction::KillPane => {
            wm.close_pane();
        }
        ContextMenuAction::SplitHorizontal => {
            wm.split_horizontal();
        }
        ContextMenuAction::SplitVertical => {
            wm.split_vertical();
        }
        ContextMenuAction::ToggleZoom => {
            wm.toggle_zoom();
        }
        ContextMenuAction::Cancel => {
            // Do nothing
        }
    }
}

/// Main event loop
#[cfg(windows)]
fn run_main_loop(session: &mut Session, renderer: &mut Renderer) -> anyhow::Result<()> {
    let poll_timeout = Duration::from_millis(10);

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
                renderer.render(&session.state)?;
                session.state.active_screen_mut().clear_dirty();
            }
            Ok(false) => {
                // No output, check again
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

        // Process input events
        if event::poll(poll_timeout)? {
            let evt = event::read()?;
            // Log all events to debug file
            renderer.log_mouse_event(&format!("Event received: {:?}", evt));
            
            match evt {
                Event::Key(key_event) => {
                    // Only process key press events
                    if key_event.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Handle scrollback keys (Shift+PageUp/PageDown)
                    if key_event.modifiers.contains(KeyModifiers::SHIFT) {
                        match key_event.code {
                            KeyCode::PageUp => {
                                let screen = session.state.active_screen_mut();
                                screen.scroll_view_up(10);
                                renderer.render(&session.state)?;
                                continue;
                            }
                            KeyCode::PageDown => {
                                let screen = session.state.active_screen_mut();
                                screen.scroll_view_down(10);
                                renderer.render(&session.state)?;
                                continue;
                            }
                            KeyCode::Home => {
                                // Scroll to top of history
                                let screen = session.state.active_screen_mut();
                                let max = screen.scrollback.len();
                                screen.scroll_offset = max;
                                screen.mark_all_dirty();
                                renderer.render(&session.state)?;
                                continue;
                            }
                            KeyCode::End => {
                                // Scroll to bottom (live)
                                let screen = session.state.active_screen_mut();
                                screen.scroll_to_bottom();
                                renderer.render(&session.state)?;
                                continue;
                            }
                            // Shift+Arrow keys for selection
                            KeyCode::Left | KeyCode::Right | KeyCode::Up | KeyCode::Down => {
                                handle_selection_key(&mut session.state, key_event.code);
                                session.state.active_screen_mut().full_redraw = true;
                                renderer.render(&session.state)?;
                                session.state.active_screen_mut().clear_dirty();
                                continue;
                            }
                            _ => {}
                        }
                    }

                    // Ctrl+Shift+C to copy selection
                    if key_event.modifiers.contains(KeyModifiers::CONTROL) 
                        && key_event.modifiers.contains(KeyModifiers::SHIFT)
                        && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('C'))
                    {
                        if let Some(text) = session.state.get_selected_text() {
                            if !text.is_empty() {
                                #[cfg(windows)]
                                {
                                    let _ = copy_to_clipboard_windows(&text);
                                }
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
                    if let Some(bytes) = KeyMapper::map(&key_event, &session.state.modes) {
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
                    // Return to live view on paste
                    session.state.active_screen_mut().scroll_to_bottom();
                    
                    // Handle paste
                    let bytes = if session.state.modes.bracketed_paste {
                        format!("\x1b[200~{}\x1b[201~", text).into_bytes()
                    } else {
                        text.into_bytes()
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
                                    
                                    // Also try Windows clipboard
                                    #[cfg(windows)]
                                    {
                                        let _ = copy_to_clipboard_windows(&text);
                                    }
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

                _ => {}
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

/// Demo mode for non-Windows platforms
#[cfg(not(windows))]
fn run_demo() -> anyhow::Result<()> {
    use crate::core::term::TerminalState;
    use crate::ui::DebugRenderer;

    println!("=== wtmux Demo Mode ===\n");

    // Create a terminal state
    let mut state = TerminalState::new(80, 24);

    // Simulate some output
    let demo_output = concat!(
        "\x1b[32mWelcome to wtmux!\x1b[0m\r\n",
        "\r\n",
        "This is a \x1b[1mbold\x1b[0m and \x1b[4munderlined\x1b[0m text.\r\n",
        "Colors: \x1b[31mRed\x1b[0m \x1b[32mGreen\x1b[0m \x1b[34mBlue\x1b[0m\r\n",
        "\r\n",
        "日本語テスト: こんにちは世界！\r\n",
        "\r\n",
        "\x1b[7mInverse text\x1b[0m\r\n",
        "\r\n",
        "PS C:\\Users\\demo> \x1b[?25h",
    );

    // Create a session and feed the demo output
    let mut session = Session::new(1, 80, 24);
    session.feed_bytes(demo_output.as_bytes());

    // Render to debug output
    println!("{}", DebugRenderer::render(&session.state));

    println!("\nDemo complete. Build on Windows to use ConPTY.");
    Ok(())
}
