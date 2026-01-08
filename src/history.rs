//! Command history for wtmux
//!
//! Provides command history storage, search, and selection functionality.

use std::fs;
use std::path::PathBuf;

/// Maximum number of history entries
const HISTORY_LIMIT: usize = 1000;

/// A single history entry
#[derive(Clone, Debug)]
pub struct HistoryEntry {
    /// The command text
    pub command: String,
    /// Unix timestamp
    pub timestamp: u64,
}

/// Command history storage
pub struct CommandHistory {
    /// All history entries (newest last)
    entries: Vec<HistoryEntry>,
    /// File path for persistence
    file_path: Option<PathBuf>,
    /// Maximum entries
    max_entries: usize,
}

impl CommandHistory {
    /// Create a new command history
    pub fn new() -> Self {
        let file_path = Self::get_history_path();
        let mut history = Self {
            entries: Vec::new(),
            file_path,
            max_entries: HISTORY_LIMIT,
        };
        history.load();
        history
    }

    /// Get history file path
    fn get_history_path() -> Option<PathBuf> {
        if let Some(home) = home_dir() {
            let wtmux_dir = home.join(".wtmux");
            if !wtmux_dir.exists() {
                let _ = fs::create_dir_all(&wtmux_dir);
            }
            return Some(wtmux_dir.join("history"));
        }
        None
    }

    /// Load history from file
    fn load(&mut self) {
        if let Some(ref path) = self.file_path {
            if path.exists() {
                if let Ok(content) = fs::read_to_string(path) {
                    for line in content.lines() {
                        if let Some((ts_str, cmd)) = line.split_once(';') {
                            if let Ok(timestamp) = ts_str.parse::<u64>() {
                                self.entries.push(HistoryEntry {
                                    command: cmd.to_string(),
                                    timestamp,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// Save history to file
    fn save(&self) {
        if let Some(ref path) = self.file_path {
            let content: String = self.entries
                .iter()
                .map(|e| format!("{};{}", e.timestamp, e.command))
                .collect::<Vec<_>>()
                .join("\n");
            let _ = fs::write(path, content);
        }
    }

    /// Add a command to history
    pub fn add(&mut self, command: String) {
        // Skip empty or whitespace-only commands
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return;
        }

        // Skip if same as last command (dedup consecutive)
        if let Some(last) = self.entries.last() {
            if last.command == trimmed {
                return;
            }
        }

        // Skip sensitive commands
        if Self::is_sensitive(trimmed) {
            return;
        }

        // Get current timestamp
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        self.entries.push(HistoryEntry {
            command: trimmed.to_string(),
            timestamp,
        });

        // Trim if exceeding limit
        while self.entries.len() > self.max_entries {
            self.entries.remove(0);
        }

        self.save();
    }

    /// Check if command is sensitive (shouldn't be saved)
    fn is_sensitive(command: &str) -> bool {
        let lower = command.to_lowercase();
        let sensitive_patterns = [
            "password", "passwd", "secret", "token", "api_key", "apikey",
            "credential", "auth", "login", "ssh-add", "gpg",
        ];
        sensitive_patterns.iter().any(|p| lower.contains(p))
    }

    /// Search history by query (newest first)
    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .iter()
            .rev() // newest first
            .filter(|e| e.command.to_lowercase().contains(&query_lower))
            .collect()
    }

    /// Get recent history (newest first)
    pub fn recent(&self, count: usize) -> Vec<&HistoryEntry> {
        self.entries.iter().rev().take(count).collect()
    }

    /// Get entry count
    #[allow(dead_code)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Strip prompt from command line
pub fn strip_prompt(line: &str) -> String {
    let line = line.trim();
    
    // Common prompt patterns to strip
    // cmd.exe: "C:\path>"
    // PowerShell: "PS C:\path>"
    // bash/zsh: "user@host:path$ " or "$ " or "# "
    // Python: ">>> " or "... "
    
    // Try to find prompt ending patterns
    let prompt_endings = [
        ">",    // cmd.exe, PowerShell
        "$ ",   // bash/zsh user
        "# ",   // bash/zsh root
        ">>> ", // Python REPL
        "... ", // Python continuation
        "]: ",  // some custom prompts
    ];
    
    for ending in prompt_endings {
        if let Some(pos) = line.rfind(ending) {
            // Check if this looks like a prompt (not too far into the line)
            // Prompts are typically at the start, within first ~60 chars
            if pos < 60 {
                let after = &line[pos + ending.len()..];
                if !after.is_empty() {
                    return after.trim().to_string();
                }
            }
        }
    }
    
    // If no prompt found, return as-is (might be a continuation)
    line.to_string()
}

/// History selector UI
pub struct HistorySelector {
    /// Command history
    pub history: CommandHistory,
    /// Current search query
    pub query: String,
    /// Filtered results (command strings)
    pub results: Vec<String>,
    /// Currently selected index
    pub selected: usize,
    /// Whether the selector is visible
    pub visible: bool,
    /// Scroll offset
    pub scroll_offset: usize,
    /// Maximum visible items
    pub max_visible: usize,
}

impl HistorySelector {
    pub fn new() -> Self {
        let history = CommandHistory::new();
        
        let mut selector = Self {
            history,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
            visible: false,
            scroll_offset: 0,
            max_visible: 10,
        };
        selector.update_results();
        selector
    }

    /// Show the selector
    pub fn show(&mut self) {
        self.visible = true;
        self.query.clear();
        self.selected = 0;
        self.scroll_offset = 0;
        self.update_results();
    }

    /// Hide the selector
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Update results based on query
    pub fn update_results(&mut self) {
        self.results.clear();
        
        // Track seen commands to avoid duplicates
        let mut seen = std::collections::HashSet::new();
        
        if self.query.is_empty() {
            // Show recent history
            for entry in self.history.recent(100) {
                if seen.insert(entry.command.clone()) {
                    self.results.push(entry.command.clone());
                }
            }
        } else {
            // Search history
            for entry in self.history.search(&self.query) {
                if seen.insert(entry.command.clone()) {
                    self.results.push(entry.command.clone());
                }
            }
        }
        
        // Reset selection if out of bounds
        if self.selected >= self.results.len() && !self.results.is_empty() {
            self.selected = self.results.len() - 1;
        }
        if self.results.is_empty() {
            self.selected = 0;
        }
        self.adjust_scroll();
    }

    /// Add character to query
    pub fn input_char(&mut self, ch: char) {
        self.query.push(ch);
        self.selected = 0;
        self.scroll_offset = 0;
        self.update_results();
    }

    /// Remove last character
    pub fn backspace(&mut self) {
        self.query.pop();
        self.update_results();
    }

    /// Move selection up
    pub fn select_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.adjust_scroll();
        }
    }

    /// Move selection down
    pub fn select_down(&mut self) {
        if !self.results.is_empty() && self.selected + 1 < self.results.len() {
            self.selected += 1;
            self.adjust_scroll();
        }
    }

    /// Select by number (1-9)
    pub fn select_number(&mut self, num: usize) -> Option<String> {
        let index = num.saturating_sub(1);
        if index < self.results.len() {
            self.selected = index;
            return self.confirm();
        }
        None
    }

    /// Adjust scroll offset
    fn adjust_scroll(&mut self) {
        if self.selected < self.scroll_offset {
            self.scroll_offset = self.selected;
        } else if self.selected >= self.scroll_offset + self.max_visible {
            self.scroll_offset = self.selected - self.max_visible + 1;
        }
    }

    /// Confirm selection
    pub fn confirm(&mut self) -> Option<String> {
        if let Some(command) = self.results.get(self.selected).cloned() {
            self.hide();
            return Some(command);
        }
        None
    }

    /// Get visible items for rendering
    /// Returns: (display_index, command, is_selected)
    pub fn visible_items(&self) -> Vec<(usize, &str, bool)> {
        self.results
            .iter()
            .enumerate()
            .skip(self.scroll_offset)
            .take(self.max_visible)
            .map(|(idx, cmd)| {
                let display_idx = idx - self.scroll_offset;
                let is_selected = idx == self.selected;
                (display_idx, cmd.as_str(), is_selected)
            })
            .collect()
    }

    /// Add command to history
    pub fn add_to_history(&mut self, command: String) {
        self.history.add(command);
    }

    /// Check if has any history
    #[allow(dead_code)]
    pub fn has_history(&self) -> bool {
        !self.history.entries.is_empty()
    }
}

// Get home directory
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}
