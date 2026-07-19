//! Pane - A single terminal pane within a tab

use std::time::{Duration, Instant};

use crate::core::session::Session;

/// Unique identifier for a pane
pub type PaneId = u64;

/// Why a pane is flagged as needing attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// The program rang the terminal bell (or sent an OSC 9 notification)
    Bell,
    /// The pane produced output while unfocused and then went quiet —
    /// a background agent likely finished or is waiting for input
    Quiet,
}

/// Coarse agent state, herdr-style: what is the program in this pane doing?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentState {
    /// Nothing notable: at a shell prompt / no tracked activity yet
    Idle,
    /// Output is flowing
    Working,
    /// Waiting on the user: bell received, or output stopped on something
    /// that looks like a question / permission prompt
    Blocked,
    /// Output stopped after a burst of work (or the process exited)
    Done,
}

impl AgentState {
    /// Short display label for the dashboard / status bar.
    pub fn label(&self) -> &'static str {
        match self {
            AgentState::Idle => "IDLE",
            AgentState::Working => "WORKING",
            AgentState::Blocked => "BLOCKED",
            AgentState::Done => "DONE",
        }
    }
}

/// What the quiet-transition heuristic saw at the cursor when output stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptHint {
    /// Nothing recognizable — treat the stop as "finished working"
    None,
    /// An ordinary shell prompt — the pane is simply idle
    ShellPrompt,
    /// A question / permission prompt — the program wants input
    Question,
}

/// Substrings (lowercase) that mark a question or permission prompt.
/// Matched against the last few lines above the cursor when a pane's
/// output goes quiet.
const QUESTION_MARKERS: &[&str] = &[
    "[y/n",
    "(y/n",
    "y/n]",
    "y/n)",
    "yes/no",
    "do you want",
    "would you like",
    "press enter",
    "press any key",
    "waiting for input",
    "waiting for your",
    "proceed?",
    "continue?",
    "confirm",
    "permission",
];

/// Inspect the screen around the cursor to classify why output stopped.
///
/// Looks at the cursor line and up to two non-empty lines above it.
pub fn scan_prompt_hint(state: &crate::core::term::TerminalState) -> PromptHint {
    let screen = state.active_screen();
    let cursor_row = state.active_cursor().row as usize;

    // Collect up to 3 non-empty lines ending at the cursor line
    let mut lines: Vec<String> = Vec::new();
    let mut row = cursor_row as isize;
    while row >= 0 && lines.len() < 3 {
        if let Some(r) = screen.rows.get(row as usize) {
            let text: String = r
                .cells
                .iter()
                .filter(|c| !c.is_continuation())
                .map(|c| c.grapheme.as_str())
                .collect();
            let text = text.trim_end().to_string();
            if !text.is_empty() {
                lines.push(text);
            } else if !lines.is_empty() {
                // Stop at the first gap above collected text
                break;
            }
        }
        row -= 1;
    }
    let Some(last_line) = lines.first() else {
        return PromptHint::None;
    };

    let joined = lines.join(" ").to_lowercase();
    if QUESTION_MARKERS.iter().any(|m| joined.contains(m))
        || last_line.trim_end().ends_with('?')
    {
        return PromptHint::Question;
    }

    // A bare shell prompt: short-ish line ending in a prompt character
    let trimmed = last_line.trim_end();
    if trimmed
        .chars()
        .last()
        .is_some_and(|c| matches!(c, '$' | '%' | '>' | '#' | '❯'))
    {
        return PromptHint::ShellPrompt;
    }

    PromptHint::None
}

/// Per-pane activity tracking for the agent-multiplexing monitor.
///
/// States surfaced to the UI:
/// - **busy**: output arrived within the quiet threshold (`*` in the tab bar)
/// - **attention**: bell received while unfocused, or unfocused output went
///   quiet (`!` in the tab bar, highlighted border). Cleared when the pane
///   regains focus.
#[derive(Debug)]
pub struct PaneActivity {
    /// When output last arrived
    last_output: Option<Instant>,
    /// Output arrived while the pane was unfocused (candidate for the
    /// busy→quiet attention transition)
    unfocused_output: bool,
    /// Pending attention flag
    attention: Option<Attention>,
    /// Busy state as last reported to the UI (to detect display changes)
    displayed_busy: bool,
    /// herdr-style coarse state (tracked for focused panes too)
    state: AgentState,
}

impl Default for PaneActivity {
    fn default() -> Self {
        Self {
            last_output: None,
            unfocused_output: false,
            attention: None,
            displayed_busy: false,
            state: AgentState::Idle,
        }
    }
}

impl PaneActivity {
    /// Record that output arrived on this pane.
    pub fn note_output(&mut self, focused: bool) {
        self.last_output = Some(Instant::now());
        self.state = AgentState::Working;
        if !focused {
            self.unfocused_output = true;
        }
    }

    /// Record a bell: the program is asking for the user. Bells on the
    /// focused pane don't raise the attention flag — the user is already
    /// looking at it — but the state still becomes Blocked.
    pub fn note_bell(&mut self, focused: bool) {
        self.state = AgentState::Blocked;
        if !focused {
            self.attention = Some(Attention::Bell);
        }
    }

    /// Record that the pane's process exited.
    pub fn note_exited(&mut self) {
        self.state = AgentState::Done;
    }

    /// Advance time-based transitions. `hint` is what the screen showed at
    /// the cursor (used to classify a quiet stop). Returns `true` when the
    /// state shown in the UI changed.
    pub fn tick(&mut self, focused: bool, quiet_threshold: Duration, hint: PromptHint) -> bool {
        let mut changed = false;

        // Focus acknowledges any pending attention.
        if focused && (self.attention.is_some() || self.unfocused_output) {
            self.attention = None;
            self.unfocused_output = false;
            changed = true;
        }

        let busy = self
            .last_output
            .map(|t| t.elapsed() < quiet_threshold)
            .unwrap_or(false);

        // Output stopped: classify what the program is now doing.
        if !busy && self.state == AgentState::Working {
            self.state = match hint {
                PromptHint::Question => AgentState::Blocked,
                PromptHint::ShellPrompt => AgentState::Idle,
                PromptHint::None => AgentState::Done,
            };
            // A shell returning to its prompt is not noteworthy; a blocked
            // or finished background program is.
            if !focused && self.unfocused_output && self.state != AgentState::Idle {
                self.attention.get_or_insert(Attention::Quiet);
            }
            self.unfocused_output = false;
            changed = true;
        }

        if busy != self.displayed_busy {
            self.displayed_busy = busy;
            changed = true;
        }

        changed
    }

    /// Pending attention flag, if any.
    pub fn attention(&self) -> Option<Attention> {
        self.attention
    }

    /// Whether output arrived within the quiet threshold (as of last `tick`).
    pub fn is_busy(&self) -> bool {
        self.displayed_busy
    }

    /// Current herdr-style state.
    pub fn state(&self) -> AgentState {
        self.state
    }
}

/// A single pane containing a terminal session
pub struct Pane {
    /// Unique identifier
    pub id: PaneId,
    /// Terminal session
    pub session: Session,
    /// Position (column, row) in the parent container
    pub x: u16,
    pub y: u16,
    /// Size (width, height)
    pub width: u16,
    pub height: u16,
    /// Whether this pane is focused
    pub focused: bool,
    /// Border style
    pub border: BorderStyle,
    /// Title (optional override)
    pub title: Option<String>,
    /// Activity monitor state (busy / needs-attention)
    pub activity: PaneActivity,
}

/// Border drawing style
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum BorderStyle {
    None,
    Single,
    Double,
    Rounded,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::Single
    }
}

impl Pane {
    /// Create a new pane with border (default)
    pub fn new(id: PaneId, cols: u16, rows: u16) -> Self {
        // Calculate inner size (accounting for border)
        // Default border is Single, so subtract 2 from each dimension
        let inner_cols = if cols > 2 { cols - 2 } else { 1 };
        let inner_rows = if rows > 2 { rows - 2 } else { 1 };
        
        Self {
            id,
            session: Session::new(id, inner_cols, inner_rows),
            x: 0,
            y: 0,
            width: cols,
            height: rows,
            focused: false,
            border: BorderStyle::default(),
            title: None,
            activity: PaneActivity::default(),
        }
    }

    /// Create a new pane without border (full size)
    pub fn new_without_border(id: PaneId, cols: u16, rows: u16) -> Self {
        Self {
            id,
            session: Session::new(id, cols, rows),
            x: 0,
            y: 0,
            width: cols,
            height: rows,
            focused: false,
            border: BorderStyle::None,
            title: None,
            activity: PaneActivity::default(),
        }
    }

    /// Get the inner dimensions (excluding border)
    pub fn inner_size(&self) -> (u16, u16) {
        match self.border {
            BorderStyle::None => (self.width, self.height),
            _ => {
                let w = if self.width > 2 { self.width - 2 } else { 1 };
                let h = if self.height > 2 { self.height - 2 } else { 1 };
                (w, h)
            }
        }
    }

    /// Get the inner position (excluding border)
    pub fn inner_pos(&self) -> (u16, u16) {
        match self.border {
            BorderStyle::None => (self.x, self.y),
            _ => (self.x + 1, self.y + 1),
        }
    }

    /// Apply geometry (border, position, size) in a consistent order
    /// This is the single entry point for geometry changes
    pub(crate) fn apply_geometry(&mut self, x: u16, y: u16, width: u16, height: u16, border: BorderStyle) {
        // Order is important: border affects inner_size calculation
        self.border = border;
        self.move_to(x, y);
        self.resize(width, height);
    }

    /// Resize the pane
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        let (inner_w, inner_h) = self.inner_size();
        if let Err(e) = self.session.resize(inner_w, inner_h) {
            eprintln!(
                "Pane {} resize failed: outer={}x{}, inner={}x{}: {}",
                self.id, width, height, inner_w, inner_h, e
            );
        }
    }

    /// Move the pane
    pub fn move_to(&mut self, x: u16, y: u16) {
        self.x = x;
        self.y = y;
    }

    /// Get display title
    pub fn display_title(&self) -> String {
        if let Some(ref title) = self.title {
            title.clone()
        } else {
            format!("Pane {}", self.id)
        }
    }

    /// Check if a position is inside this pane
    pub fn contains(&self, col: u16, row: u16) -> bool {
        col >= self.x && col < self.x + self.width &&
        row >= self.y && row < self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::term::TerminalState;

    const QUIET: Duration = Duration::from_millis(0);

    #[test]
    fn unfocused_output_going_quiet_raises_attention_and_marks_done() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        // Zero threshold: the pane counts as quiet immediately
        assert!(a.tick(false, QUIET, PromptHint::None));
        assert_eq!(a.attention(), Some(Attention::Quiet));
        assert_eq!(a.state(), AgentState::Done);
    }

    #[test]
    fn focused_output_going_quiet_stays_calm_but_tracks_state() {
        let mut a = PaneActivity::default();
        a.note_output(true);
        a.tick(true, QUIET, PromptHint::None);
        assert_eq!(a.attention(), None);
        assert_eq!(a.state(), AgentState::Done);
    }

    #[test]
    fn question_prompt_marks_blocked() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        a.tick(false, QUIET, PromptHint::Question);
        assert_eq!(a.state(), AgentState::Blocked);
        assert_eq!(a.attention(), Some(Attention::Quiet));
    }

    #[test]
    fn shell_prompt_marks_idle_without_attention() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        a.tick(false, QUIET, PromptHint::ShellPrompt);
        assert_eq!(a.state(), AgentState::Idle);
        assert_eq!(a.attention(), None, "returning to a shell prompt is not noteworthy");
    }

    #[test]
    fn bell_on_unfocused_pane_raises_attention_and_wins_over_quiet() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        a.note_bell(false);
        a.tick(false, QUIET, PromptHint::None);
        assert_eq!(a.attention(), Some(Attention::Bell));
    }

    #[test]
    fn bell_marks_blocked_even_when_focused() {
        let mut a = PaneActivity::default();
        a.note_bell(true);
        a.tick(true, QUIET, PromptHint::None);
        assert_eq!(a.attention(), None);
        assert_eq!(a.state(), AgentState::Blocked);
    }

    #[test]
    fn regaining_focus_clears_attention() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        a.tick(false, QUIET, PromptHint::None);
        assert_eq!(a.attention(), Some(Attention::Quiet));

        assert!(a.tick(true, QUIET, PromptHint::None));
        assert_eq!(a.attention(), None);
        // ...and it does not come back on the next tick
        a.tick(false, QUIET, PromptHint::None);
        assert_eq!(a.attention(), None);
    }

    #[test]
    fn busy_state_follows_recent_output() {
        let mut a = PaneActivity::default();
        assert!(!a.is_busy());
        a.note_output(true);
        assert!(a.tick(true, Duration::from_secs(60), PromptHint::None));
        assert!(a.is_busy());
        assert_eq!(a.state(), AgentState::Working);
    }

    #[test]
    fn new_output_returns_a_blocked_pane_to_working() {
        let mut a = PaneActivity::default();
        a.note_bell(false);
        assert_eq!(a.state(), AgentState::Blocked);
        a.note_output(false);
        assert_eq!(a.state(), AgentState::Working);
    }

    fn state_with_text(lines: &[&str]) -> TerminalState {
        let mut state = TerminalState::new(80, 24);
        let mut session = crate::core::session::Session::new(0, 80, 24);
        let joined = lines.join("\r\n");
        session.feed_bytes(joined.as_bytes());
        std::mem::swap(&mut state, &mut session.state);
        state
    }

    #[test]
    fn scan_detects_permission_prompts() {
        for text in [
            "Do you want to make this edit? [y/n]",
            "Overwrite file? (y/N):",
            "Press Enter to continue",
            "May I run this command?",
        ] {
            let state = state_with_text(&[text]);
            assert_eq!(
                scan_prompt_hint(&state),
                PromptHint::Question,
                "should detect: {text}"
            );
        }
    }

    #[test]
    fn scan_detects_shell_prompts() {
        for text in ["user@host:~$", "PS C:\\Users\\me>", "~/src %", "❯"] {
            let state = state_with_text(&[text]);
            assert_eq!(
                scan_prompt_hint(&state),
                PromptHint::ShellPrompt,
                "should detect: {text}"
            );
        }
    }

    #[test]
    fn scan_returns_none_for_ordinary_output() {
        let state = state_with_text(&["Compiling wtmux v2.0.0", "Finished dev profile"]);
        assert_eq!(scan_prompt_hint(&state), PromptHint::None);
    }

    #[test]
    fn scan_sees_question_above_a_menu_line() {
        // Claude Code style: question line followed by numbered options
        let state = state_with_text(&[
            "Do you want to proceed?",
            "  1. Yes",
            "  2. No",
        ]);
        assert_eq!(scan_prompt_hint(&state), PromptHint::Question);
    }
}
