//! Configurable key bindings (tmux `bind-key` equivalent).
//!
//! Every key reachable from the prefix table, plus optional prefix-less
//! ("root") keys, is expressed as a [`BoundAction`] looked up through a
//! [`BindTable`]. Defaults reproduce the historical hard-coded table, and
//! `config.toml` overlays on top of them:
//!
//! ```toml
//! unbind = ["d"]              # must precede the [bind] tables
//!
//! [bind]
//! "M-4" = "select-layout main-vertical"
//! "C-o" = "swap-pane -D"
//!
//! [bind_root]
//! "M-t" = "select-layout tiled"
//! ```

use std::collections::BTreeMap;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::wm::layout::LayoutType;
use crate::wm::SplitDirection;

/// A command bound to a key.
///
/// Split into two families by how the event loop dispatches them: most map
/// straight onto an `AppAction`, while the `Ui*` variants open a modal UI and
/// therefore need the event loop's own state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoundAction {
    /// Consume the key and do nothing (also the fallback for unbound keys).
    Noop,
    NewWindow,
    KillPane,
    KillWindow,
    SplitVertical,
    SplitHorizontal,
    NextWindow,
    PrevWindow,
    LastWindow,
    SelectWindow(usize),
    SelectPaneDir {
        direction: SplitDirection,
        forward: bool,
    },
    ResizePaneDir {
        direction: SplitDirection,
        arrow_up_or_left: bool,
    },
    NextPane,
    PrevPane,
    ToggleZoom,
    NextLayout,
    SelectLayout(LayoutType),
    ResizePane {
        grow: bool,
    },
    SwapPaneNext,
    SwapPanePrev,
    ToggleBroadcast,
    TogglePipeLog,
    NextAttention,
    ResetCursorShape,
    Paste,
    SendPrefix,
    Detach,
    // ── Actions that open a modal UI ─────────────────────────────────────
    UiRenameWindow,
    UiCopyMode,
    UiSearch,
    UiThemeSelector,
    UiWindowSelector,
    UiCommandPrompt,
    UiAgentDashboard,
    UiMessageComposer,
    UiDisplayPanes,
}

impl BoundAction {
    /// Whether this action opens a modal UI (and so must be handled by the
    /// event loop rather than `apply_app_action`).
    pub fn is_ui(self) -> bool {
        matches!(
            self,
            BoundAction::UiRenameWindow
                | BoundAction::UiCopyMode
                | BoundAction::UiSearch
                | BoundAction::UiThemeSelector
                | BoundAction::UiWindowSelector
                | BoundAction::UiCommandPrompt
                | BoundAction::UiAgentDashboard
                | BoundAction::UiMessageComposer
                | BoundAction::UiDisplayPanes
        )
    }
}

/// A key as written in a binding.
///
/// Deliberately not [`crate::config::KeyBinding`]: bindings need
/// case-sensitive character matching (so `P` and `p` are distinct keys) and
/// must ignore the Shift bit on characters, because terminals disagree on
/// whether `%` arrives with `SHIFT` set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BindKey {
    code: KeyCode,
    /// Only `CONTROL` / `ALT` for character keys; may also carry `SHIFT` for
    /// named keys such as `S-Up`.
    mods: KeyModifiers,
}

impl BindKey {
    /// Parse a tmux-ish key name: `c`, `C-o`, `M-4`, `S-Up`, `Space`, `Enter`.
    ///
    /// Accepts both `-` and `+` as separators and the long modifier spellings
    /// (`Ctrl+R`), matching [`crate::config::KeyBinding::parse`].
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }

        // A lone separator is a literal key ("-" and "+" are both bindable).
        if s == "-" || s == "+" {
            return Some(Self {
                code: KeyCode::Char(s.chars().next()?),
                mods: KeyModifiers::empty(),
            });
        }

        let parts: Vec<&str> = s
            .split(['-', '+'])
            .map(str::trim)
            .filter(|part| !part.is_empty())
            .collect();
        // "C-+" and friends: the final separator was swallowed by the split.
        let (modifier_parts, key_part) = if s.ends_with('-') || s.ends_with('+') {
            (&parts[..], s[s.len() - 1..].to_string())
        } else {
            let (last, rest) = parts.split_last()?;
            (rest, (*last).to_string())
        };

        let mut mods = KeyModifiers::empty();
        for modifier in modifier_parts {
            match modifier.to_ascii_lowercase().as_str() {
                "c" | "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
                "s" | "shift" => mods |= KeyModifiers::SHIFT,
                "a" | "alt" | "meta" | "m" => mods |= KeyModifiers::ALT,
                _ => return None,
            }
        }

        let code = match key_part.to_ascii_lowercase().as_str() {
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "pageup" | "page_up" | "ppage" => KeyCode::PageUp,
            "pagedown" | "page_down" | "npage" => KeyCode::PageDown,
            "home" => KeyCode::Home,
            "end" => KeyCode::End,
            "enter" | "return" => KeyCode::Enter,
            "esc" | "escape" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backspace" | "bspace" => KeyCode::Backspace,
            "delete" | "del" | "dc" => KeyCode::Delete,
            "insert" | "ic" => KeyCode::Insert,
            "space" => KeyCode::Char(' '),
            lowered if lowered.starts_with('f') && lowered.len() <= 3 => {
                match lowered[1..].parse::<u8>() {
                    Ok(n) if (1..=12).contains(&n) => KeyCode::F(n),
                    _ => return None,
                }
            }
            _ if key_part.chars().count() == 1 => KeyCode::Char(key_part.chars().next()?),
            _ => return None,
        };

        // For characters the case already encodes Shift, so fold `S-p` into
        // `P` and drop the bit; keeping it would never match, since terminals
        // are inconsistent about reporting Shift alongside a shifted glyph.
        if let KeyCode::Char(ch) = code {
            let ch = if mods.contains(KeyModifiers::SHIFT) {
                ch.to_ascii_uppercase()
            } else {
                ch
            };
            return Some(Self {
                code: KeyCode::Char(ch),
                mods: mods & (KeyModifiers::CONTROL | KeyModifiers::ALT),
            });
        }

        Some(Self { code, mods })
    }

    pub fn matches(&self, event: &KeyEvent) -> bool {
        match (self.code, event.code) {
            (KeyCode::Char(expected), KeyCode::Char(actual)) => {
                let relevant = KeyModifiers::CONTROL | KeyModifiers::ALT;
                if event.modifiers & relevant != self.mods & relevant {
                    return false;
                }
                // Ctrl+<letter> is reported with inconsistent case across
                // terminals, so compare case-insensitively there only.
                if self.mods.contains(KeyModifiers::CONTROL) {
                    expected.eq_ignore_ascii_case(&actual)
                } else {
                    expected == actual
                }
            }
            (expected, actual) => {
                let relevant =
                    KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SHIFT;
                expected == actual && event.modifiers & relevant == self.mods
            }
        }
    }

    pub fn display_name(&self) -> String {
        let mut parts = Vec::new();
        if self.mods.contains(KeyModifiers::CONTROL) {
            parts.push("Ctrl".to_string());
        }
        if self.mods.contains(KeyModifiers::ALT) {
            parts.push("Alt".to_string());
        }
        if self.mods.contains(KeyModifiers::SHIFT) {
            parts.push("Shift".to_string());
        }
        parts.push(match self.code {
            KeyCode::Char(' ') => "Space".to_string(),
            KeyCode::Char(ch) => ch.to_string(),
            KeyCode::F(n) => format!("F{n}"),
            other => format!("{other:?}"),
        });
        parts.join("+")
    }
}

/// Parse the command half of a binding, e.g. `"select-layout main-vertical"`.
///
/// Command names follow tmux where an equivalent exists; wtmux-only actions
/// use their own names. Returns a human-readable message on failure so the
/// caller can report which line of `config.toml` is wrong.
pub fn parse_action(spec: &str) -> Result<BoundAction, String> {
    let mut parts = spec.split_whitespace();
    let name = parts.next().ok_or_else(|| "empty command".to_string())?;
    let args: Vec<&str> = parts.collect();

    let action = match name {
        "new-window" | "neww" => BoundAction::NewWindow,
        "kill-pane" | "killp" => BoundAction::KillPane,
        "kill-window" | "killw" => BoundAction::KillWindow,
        "split-window" | "splitw" => {
            if args.contains(&"-h") {
                BoundAction::SplitHorizontal
            } else {
                BoundAction::SplitVertical
            }
        }
        "next-window" | "next" => BoundAction::NextWindow,
        "previous-window" | "prev" => BoundAction::PrevWindow,
        "last-window" | "last" => BoundAction::LastWindow,
        "select-window" | "selectw" => {
            let index = arg_after(&args, "-t")
                .ok_or_else(|| "usage: select-window -t <index>".to_string())?;
            let index = index
                .parse::<usize>()
                .map_err(|_| format!("invalid window index: {index}"))?;
            BoundAction::SelectWindow(index)
        }
        "select-pane" | "selectp" => match direction_flag(&args) {
            // `-L`/`-U` move backwards through the layout tree, hence the flip.
            Some((direction, up_or_left)) => BoundAction::SelectPaneDir {
                direction,
                forward: !up_or_left,
            },
            None if args.iter().any(|a| *a == "-t" || *a == ":.+") => BoundAction::NextPane,
            None => {
                return Err(
                    "usage: select-pane -L|-R|-U|-D (or next-pane / previous-pane)".to_string(),
                )
            }
        },
        "next-pane" => BoundAction::NextPane,
        "previous-pane" | "last-pane" => BoundAction::PrevPane,
        "resize-pane" | "resizep" => {
            if args.contains(&"-Z") {
                BoundAction::ToggleZoom
            } else if let Some((direction, arrow_up_or_left)) = direction_flag(&args) {
                BoundAction::ResizePaneDir {
                    direction,
                    arrow_up_or_left,
                }
            } else if args.contains(&"+") {
                BoundAction::ResizePane { grow: true }
            } else if args.contains(&"-") {
                BoundAction::ResizePane { grow: false }
            } else {
                return Err("usage: resize-pane -Z | -L|-R|-U|-D | + | -".to_string());
            }
        }
        "select-layout" | "selectl" => {
            let name = args.first().ok_or_else(|| {
                "usage: select-layout <even-horizontal|even-vertical|main-horizontal|main-vertical|tiled>"
                    .to_string()
            })?;
            let layout =
                LayoutType::parse(name).ok_or_else(|| format!("unknown layout: {name}"))?;
            BoundAction::SelectLayout(layout)
        }
        "next-layout" | "nextl" => BoundAction::NextLayout,
        "swap-pane" | "swapp" => {
            if args.contains(&"-U") {
                BoundAction::SwapPanePrev
            } else {
                BoundAction::SwapPaneNext
            }
        }
        "rename-window" | "renamew" => BoundAction::UiRenameWindow,
        "copy-mode" => BoundAction::UiCopyMode,
        "search" | "search-mode" => BoundAction::UiSearch,
        "choose-window" | "choosew" => BoundAction::UiWindowSelector,
        "choose-theme" => BoundAction::UiThemeSelector,
        "command-prompt" => BoundAction::UiCommandPrompt,
        "display-panes" | "displayp" => BoundAction::UiDisplayPanes,
        "agent-dashboard" => BoundAction::UiAgentDashboard,
        "compose-message" => BoundAction::UiMessageComposer,
        "next-attention" => BoundAction::NextAttention,
        "reset-cursor" => BoundAction::ResetCursorShape,
        "pipe-pane" | "pipep" => BoundAction::TogglePipeLog,
        "paste-buffer" | "pasteb" => BoundAction::Paste,
        "send-prefix" => BoundAction::SendPrefix,
        "detach-client" | "detach" => BoundAction::Detach,
        "set" | "set-option" | "setw" | "set-window-option" => {
            match args.first().copied() {
                Some("synchronize-panes") => BoundAction::ToggleBroadcast,
                Some(other) => return Err(format!("cannot bind: set {other}")),
                None => return Err("usage: set synchronize-panes".to_string()),
            }
        }
        "synchronize-panes" => BoundAction::ToggleBroadcast,
        "none" | "nop" => BoundAction::Noop,
        other => return Err(format!("unknown command: {other}")),
    };

    Ok(action)
}

fn arg_after<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    let index = args.iter().position(|a| *a == flag)?;
    args.get(index + 1).copied()
}

/// `(direction, arrow_up_or_left)` for the tmux `-L/-R/-U/-D` flags.
fn direction_flag(args: &[&str]) -> Option<(SplitDirection, bool)> {
    for arg in args {
        return Some(match *arg {
            "-L" => (SplitDirection::Horizontal, true),
            "-R" => (SplitDirection::Horizontal, false),
            "-U" => (SplitDirection::Vertical, true),
            "-D" => (SplitDirection::Vertical, false),
            _ => continue,
        });
    }
    None
}

/// Prefix and root binding tables, defaults overlaid with user config.
pub struct BindTable {
    prefix: Vec<(BindKey, BoundAction)>,
    root: Vec<(BindKey, BoundAction)>,
    /// Problems found while parsing `config.toml`, reported once at startup.
    errors: Vec<String>,
}

impl BindTable {
    /// Build the effective tables.
    ///
    /// `prefix_char` seeds the `send-prefix` defaults so that changing
    /// `prefix_key` keeps "press it twice to send it through" working.
    pub fn build(
        prefix_char: char,
        bind: &BTreeMap<String, String>,
        bind_root: &BTreeMap<String, String>,
        unbind: &[String],
    ) -> Self {
        let mut table = Self {
            prefix: default_prefix_binds(prefix_char),
            root: Vec::new(),
            errors: Vec::new(),
        };

        table.overlay("bind", bind, false);
        table.overlay("bind_root", bind_root, true);

        for key in unbind {
            match BindKey::parse(key) {
                Some(parsed) => {
                    table.prefix.retain(|(k, _)| *k != parsed);
                    table.root.retain(|(k, _)| *k != parsed);
                }
                None => table.errors.push(format!("unbind: invalid key {key:?}")),
            }
        }

        table
    }

    fn overlay(&mut self, section: &str, entries: &BTreeMap<String, String>, root: bool) {
        for (key, spec) in entries {
            let Some(parsed) = BindKey::parse(key) else {
                self.errors
                    .push(format!("[{section}] invalid key {key:?}"));
                continue;
            };

            let target = if root { &mut self.root } else { &mut self.prefix };

            // An empty value unbinds, mirroring `unbind-key`.
            if spec.trim().is_empty() {
                target.retain(|(k, _)| *k != parsed);
                continue;
            }

            match parse_action(spec) {
                Ok(action) => {
                    target.retain(|(k, _)| *k != parsed);
                    target.push((parsed, action));
                }
                Err(e) => self
                    .errors
                    .push(format!("[{section}] {key:?} = {spec:?}: {e}")),
            }
        }
    }

    /// Action bound to `event` while prefix mode is active.
    ///
    /// Falls back to the unmodified key when Ctrl is held and nothing matches,
    /// so that keeping Ctrl down through the whole sequence (`C-b C-n`) still
    /// reaches the bare binding — the behaviour before bindings became
    /// configurable. An explicit `C-` binding is found by the strict pass and
    /// therefore always wins.
    pub fn lookup_prefix(&self, event: &KeyEvent) -> Option<BoundAction> {
        if let Some(action) = lookup(&self.prefix, event) {
            return Some(action);
        }
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            let mut relaxed = *event;
            relaxed.modifiers.remove(KeyModifiers::CONTROL);
            return lookup(&self.prefix, &relaxed);
        }
        None
    }

    /// Action bound to `event` with no prefix (empty unless configured).
    pub fn lookup_root(&self, event: &KeyEvent) -> Option<BoundAction> {
        lookup(&self.root, event)
    }

    /// True when no root bindings exist, so the event loop can skip the scan.
    pub fn root_is_empty(&self) -> bool {
        self.root.is_empty()
    }

    pub fn errors(&self) -> &[String] {
        &self.errors
    }

    /// All bindings as `(scope, key, command)` for `list-keys`.
    pub fn describe(&self) -> Vec<(&'static str, String, String)> {
        let mut rows: Vec<(&'static str, String, String)> = self
            .prefix
            .iter()
            .map(|(k, a)| ("prefix", k.display_name(), describe_action(*a)))
            .chain(
                self.root
                    .iter()
                    .map(|(k, a)| ("root", k.display_name(), describe_action(*a))),
            )
            .collect();
        rows.sort_by(|a, b| a.0.cmp(b.0).then_with(|| a.1.cmp(&b.1)));
        rows
    }
}

/// Later entries win, so a user override pushed after a default shadows it
/// even if the default was somehow left in place.
fn lookup(table: &[(BindKey, BoundAction)], event: &KeyEvent) -> Option<BoundAction> {
    table
        .iter()
        .rev()
        .find(|(key, _)| key.matches(event))
        .map(|(_, action)| *action)
}

fn describe_action(action: BoundAction) -> String {
    use BoundAction as B;
    match action {
        B::Noop => "none".into(),
        B::NewWindow => "new-window".into(),
        B::KillPane => "kill-pane".into(),
        B::KillWindow => "kill-window".into(),
        B::SplitVertical => "split-window".into(),
        B::SplitHorizontal => "split-window -h".into(),
        B::NextWindow => "next-window".into(),
        B::PrevWindow => "previous-window".into(),
        B::LastWindow => "last-window".into(),
        B::SelectWindow(n) => format!("select-window -t {n}"),
        B::SelectPaneDir { direction, forward } => {
            format!("select-pane {}", dir_flag(direction, !forward))
        }
        B::ResizePaneDir {
            direction,
            arrow_up_or_left,
        } => format!("resize-pane {}", dir_flag(direction, arrow_up_or_left)),
        B::NextPane => "next-pane".into(),
        B::PrevPane => "previous-pane".into(),
        B::ToggleZoom => "resize-pane -Z".into(),
        B::NextLayout => "next-layout".into(),
        B::SelectLayout(layout) => format!("select-layout {}", layout.name()),
        B::ResizePane { grow } => {
            format!("resize-pane {}", if grow { "+" } else { "-" })
        }
        B::SwapPaneNext => "swap-pane -D".into(),
        B::SwapPanePrev => "swap-pane -U".into(),
        B::ToggleBroadcast => "set synchronize-panes".into(),
        B::TogglePipeLog => "pipe-pane".into(),
        B::NextAttention => "next-attention".into(),
        B::ResetCursorShape => "reset-cursor".into(),
        B::Paste => "paste-buffer".into(),
        B::SendPrefix => "send-prefix".into(),
        B::Detach => "detach-client".into(),
        B::UiRenameWindow => "rename-window".into(),
        B::UiCopyMode => "copy-mode".into(),
        B::UiSearch => "search".into(),
        B::UiThemeSelector => "choose-theme".into(),
        B::UiWindowSelector => "choose-window".into(),
        B::UiCommandPrompt => "command-prompt".into(),
        B::UiAgentDashboard => "agent-dashboard".into(),
        B::UiMessageComposer => "compose-message".into(),
        B::UiDisplayPanes => "display-panes".into(),
    }
}

fn dir_flag(direction: SplitDirection, up_or_left: bool) -> &'static str {
    match (direction, up_or_left) {
        (SplitDirection::Horizontal, true) => "-L",
        (SplitDirection::Horizontal, false) => "-R",
        (SplitDirection::Vertical, true) => "-U",
        (SplitDirection::Vertical, false) => "-D",
    }
}

/// The built-in prefix table — the tmux-compatible defaults wtmux has always
/// shipped, now expressed as data so `config.toml` can override any row.
fn default_prefix_binds(prefix_char: char) -> Vec<(BindKey, BoundAction)> {
    use BoundAction as B;
    use SplitDirection::{Horizontal, Vertical};

    let mut binds: Vec<(&str, BoundAction)> = vec![
        ("Esc", B::Noop),
        ("c", B::NewWindow),
        ("x", B::KillPane),
        ("&", B::KillWindow),
        ("\"", B::SplitVertical),
        ("%", B::SplitHorizontal),
        ("n", B::NextWindow),
        ("p", B::PrevWindow),
        ("l", B::LastWindow),
        ("o", B::NextPane),
        (";", B::PrevPane),
        ("e", B::ToggleBroadcast),
        ("P", B::TogglePipeLog),
        ("a", B::NextAttention),
        ("r", B::ResetCursorShape),
        ("z", B::ToggleZoom),
        ("Space", B::NextLayout),
        ("+", B::ResizePane { grow: true }),
        ("=", B::ResizePane { grow: true }),
        ("-", B::ResizePane { grow: false }),
        ("}", B::SwapPaneNext),
        ("{", B::SwapPanePrev),
        ("d", B::Detach),
        (",", B::UiRenameWindow),
        ("[", B::UiCopyMode),
        ("/", B::UiSearch),
        ("t", B::UiThemeSelector),
        ("w", B::UiWindowSelector),
        (":", B::UiCommandPrompt),
        ("g", B::UiAgentDashboard),
        ("m", B::UiMessageComposer),
        ("q", B::UiDisplayPanes),
        (
            "Left",
            B::SelectPaneDir {
                direction: Horizontal,
                forward: false,
            },
        ),
        (
            "Right",
            B::SelectPaneDir {
                direction: Horizontal,
                forward: true,
            },
        ),
        (
            "Up",
            B::SelectPaneDir {
                direction: Vertical,
                forward: false,
            },
        ),
        (
            "Down",
            B::SelectPaneDir {
                direction: Vertical,
                forward: true,
            },
        ),
        (
            "C-Left",
            B::ResizePaneDir {
                direction: Horizontal,
                arrow_up_or_left: true,
            },
        ),
        (
            "C-Right",
            B::ResizePaneDir {
                direction: Horizontal,
                arrow_up_or_left: false,
            },
        ),
        (
            "C-Up",
            B::ResizePaneDir {
                direction: Vertical,
                arrow_up_or_left: true,
            },
        ),
        (
            "C-Down",
            B::ResizePaneDir {
                direction: Vertical,
                arrow_up_or_left: false,
            },
        ),
    ];

    let digits: Vec<(String, BoundAction)> = (0..=9)
        .map(|n| (n.to_string(), B::SelectWindow(n)))
        .collect();

    binds
        .drain(..)
        .map(|(key, action)| (key.to_string(), action))
        .chain(digits)
        // Pressing the prefix key again sends it to the pane, both bare and
        // with Ctrl still held. Registered last so it wins over any default
        // sharing that character (e.g. `a` = next-attention when the prefix
        // key is Ctrl+A).
        .chain([
            (prefix_char.to_string(), B::SendPrefix),
            (format!("C-{prefix_char}"), B::SendPrefix),
        ])
        .filter_map(|(key, action)| BindKey::parse(&key).map(|k| (k, action)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn parses_modifier_spellings() {
        assert_eq!(
            BindKey::parse("C-o"),
            Some(BindKey {
                code: KeyCode::Char('o'),
                mods: KeyModifiers::CONTROL
            })
        );
        assert_eq!(BindKey::parse("Ctrl+O"), BindKey::parse("C-O"));
        assert_eq!(BindKey::parse("M-4"), BindKey::parse("Alt+4"));
        assert_eq!(
            BindKey::parse("Space"),
            Some(BindKey {
                code: KeyCode::Char(' '),
                mods: KeyModifiers::empty()
            })
        );
        assert!(BindKey::parse("bogus-key").is_none());
    }

    #[test]
    fn shift_folds_into_character_case() {
        assert_eq!(BindKey::parse("S-p"), BindKey::parse("P"));
        // ...but stays a real modifier on named keys.
        assert_ne!(BindKey::parse("S-Up"), BindKey::parse("Up"));
    }

    #[test]
    fn separator_keys_are_bindable() {
        assert_eq!(
            BindKey::parse("-"),
            Some(BindKey {
                code: KeyCode::Char('-'),
                mods: KeyModifiers::empty()
            })
        );
        assert_eq!(
            BindKey::parse("C-+"),
            Some(BindKey {
                code: KeyCode::Char('+'),
                mods: KeyModifiers::CONTROL
            })
        );
    }

    #[test]
    fn character_case_is_significant() {
        let upper = BindKey::parse("P").unwrap();
        assert!(upper.matches(&ev(KeyCode::Char('P'), KeyModifiers::SHIFT)));
        assert!(!upper.matches(&ev(KeyCode::Char('p'), KeyModifiers::NONE)));
    }

    #[test]
    fn shift_on_symbols_is_ignored() {
        // Terminals disagree on whether `%` carries SHIFT.
        let percent = BindKey::parse("%").unwrap();
        assert!(percent.matches(&ev(KeyCode::Char('%'), KeyModifiers::NONE)));
        assert!(percent.matches(&ev(KeyCode::Char('%'), KeyModifiers::SHIFT)));
        assert!(!percent.matches(&ev(KeyCode::Char('%'), KeyModifiers::CONTROL)));
    }

    #[test]
    fn named_keys_match_modifiers_exactly() {
        let plain = BindKey::parse("Left").unwrap();
        let ctrl = BindKey::parse("C-Left").unwrap();
        assert!(plain.matches(&ev(KeyCode::Left, KeyModifiers::NONE)));
        assert!(!plain.matches(&ev(KeyCode::Left, KeyModifiers::CONTROL)));
        assert!(ctrl.matches(&ev(KeyCode::Left, KeyModifiers::CONTROL)));
    }

    #[test]
    fn select_pane_flags_match_the_default_arrow_binds() {
        let table = BindTable::build('b', &BTreeMap::new(), &BTreeMap::new(), &[]);
        // `Left` and `select-pane -L` must mean the same thing.
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Left, KeyModifiers::NONE)),
            Some(parse_action("select-pane -L").unwrap())
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Down, KeyModifiers::NONE)),
            Some(parse_action("select-pane -D").unwrap())
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Right, KeyModifiers::CONTROL)),
            Some(parse_action("resize-pane -R").unwrap())
        );
    }

    /// Every default must round-trip through `describe` -> `parse_action`, so
    /// `list-keys` output is always valid `[bind]` syntax.
    #[test]
    fn descriptions_round_trip() {
        let table = BindTable::build('b', &BTreeMap::new(), &BTreeMap::new(), &[]);
        for (_, key, command) in table.describe() {
            let parsed = parse_action(&command)
                .unwrap_or_else(|e| panic!("{key} = {command:?} does not re-parse: {e}"));
            let bind_key = BindKey::parse(&key)
                .unwrap_or_else(|| panic!("{key:?} does not re-parse as a key"));
            assert_eq!(
                table.prefix.iter().find(|(k, _)| *k == bind_key).map(|(_, a)| *a),
                Some(parsed),
                "{key} = {command:?} round-trips to a different action",
            );
        }
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse_action("new-window").unwrap(), BoundAction::NewWindow);
        assert_eq!(
            parse_action("split-window -h").unwrap(),
            BoundAction::SplitHorizontal
        );
        assert_eq!(
            parse_action("select-layout main-vertical").unwrap(),
            BoundAction::SelectLayout(LayoutType::MainVertical)
        );
        assert_eq!(
            parse_action("resize-pane -Z").unwrap(),
            BoundAction::ToggleZoom
        );
        assert_eq!(
            parse_action("select-window -t 3").unwrap(),
            BoundAction::SelectWindow(3)
        );
        assert!(parse_action("select-layout bogus").is_err());
        assert!(parse_action("no-such-command").is_err());
    }

    #[test]
    fn defaults_reproduce_the_legacy_table() {
        let table = BindTable::build('b', &BTreeMap::new(), &BTreeMap::new(), &[]);
        assert!(table.errors().is_empty());
        assert!(table.root_is_empty());
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('c'), KeyModifiers::NONE)),
            Some(BoundAction::NewWindow)
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char(' '), KeyModifiers::NONE)),
            Some(BoundAction::NextLayout)
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('7'), KeyModifiers::NONE)),
            Some(BoundAction::SelectWindow(7))
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('b'), KeyModifiers::CONTROL)),
            Some(BoundAction::SendPrefix)
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('P'), KeyModifiers::SHIFT)),
            Some(BoundAction::TogglePipeLog)
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('p'), KeyModifiers::NONE)),
            Some(BoundAction::PrevWindow)
        );
    }

    #[test]
    fn user_bindings_override_defaults() {
        let bind = BTreeMap::from([
            ("M-4".to_string(), "select-layout main-vertical".to_string()),
            ("c".to_string(), "kill-pane".to_string()),
        ]);
        let root = BTreeMap::from([("M-t".to_string(), "select-layout tiled".to_string())]);
        let table = BindTable::build('b', &bind, &root, &[]);

        assert!(table.errors().is_empty());
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('4'), KeyModifiers::ALT)),
            Some(BoundAction::SelectLayout(LayoutType::MainVertical))
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('c'), KeyModifiers::NONE)),
            Some(BoundAction::KillPane)
        );
        assert!(!table.root_is_empty());
        assert_eq!(
            table.lookup_root(&ev(KeyCode::Char('t'), KeyModifiers::ALT)),
            Some(BoundAction::SelectLayout(LayoutType::Tiled))
        );
        // Root bindings must not leak into the prefix table.
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('t'), KeyModifiers::NONE)),
            Some(BoundAction::UiThemeSelector)
        );
    }

    #[test]
    fn holding_ctrl_through_the_prefix_still_works() {
        let bind = BTreeMap::from([("C-o".to_string(), "swap-pane -D".to_string())]);
        let table = BindTable::build('b', &bind, &BTreeMap::new(), &[]);

        // No `C-n` binding exists, so it falls back to bare `n`.
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('n'), KeyModifiers::CONTROL)),
            Some(BoundAction::NextWindow)
        );
        // An explicit C- binding beats the fallback...
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            Some(BoundAction::SwapPaneNext)
        );
        // ...and does not swallow the bare key.
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('o'), KeyModifiers::NONE)),
            Some(BoundAction::NextPane)
        );
        // Root bindings stay strict: Ctrl must not be guessed away there,
        // or ordinary shell input would start firing commands.
        let root = BTreeMap::from([("t".to_string(), "select-layout tiled".to_string())]);
        let table = BindTable::build('b', &BTreeMap::new(), &root, &[]);
        assert_eq!(
            table.lookup_root(&ev(KeyCode::Char('t'), KeyModifiers::CONTROL)),
            None
        );
    }

    #[test]
    fn unbind_removes_defaults() {
        let table = BindTable::build(
            'b',
            &BTreeMap::new(),
            &BTreeMap::new(),
            &["d".to_string(), "q".to_string()],
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('d'), KeyModifiers::NONE)),
            None
        );
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('q'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn empty_value_unbinds() {
        let bind = BTreeMap::from([("z".to_string(), String::new())]);
        let table = BindTable::build('b', &bind, &BTreeMap::new(), &[]);
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('z'), KeyModifiers::NONE)),
            None
        );
    }

    #[test]
    fn bad_entries_are_collected_not_fatal() {
        let bind = BTreeMap::from([
            ("M-9".to_string(), "no-such-command".to_string()),
            ("nope-key".to_string(), "new-window".to_string()),
            ("M-1".to_string(), "select-layout tiled".to_string()),
        ]);
        let table = BindTable::build('b', &bind, &BTreeMap::new(), &[]);
        assert_eq!(table.errors().len(), 2);
        // The valid entry still applies.
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('1'), KeyModifiers::ALT)),
            Some(BoundAction::SelectLayout(LayoutType::Tiled))
        );
    }

    #[test]
    fn prefix_key_seeds_send_prefix() {
        let table = BindTable::build('a', &BTreeMap::new(), &BTreeMap::new(), &[]);
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('a'), KeyModifiers::CONTROL)),
            Some(BoundAction::SendPrefix)
        );
        // 'a' is next-attention by default, but send-prefix is registered
        // later and therefore wins for the bare key too.
        assert_eq!(
            table.lookup_prefix(&ev(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(BoundAction::SendPrefix)
        );
    }
}
