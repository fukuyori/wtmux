//! tmux-style command prompt (`Prefix + :`) parsing.
//!
//! Parses a command line typed into the prompt into a [`PromptAction`]
//! executed by the main loop. Command names follow tmux, including the
//! common abbreviations (`splitw`, `neww`, `killp`, ...).

use crate::wm::layout::LayoutType;
use crate::wm::SplitDirection;

/// One parsed prompt command.
#[derive(Debug, Clone, PartialEq)]
pub enum PromptAction {
    /// split-window; `-h` = left/right, default = top/bottom (tmux semantics)
    Split { direction: SplitDirection },
    NewWindow,
    KillPane,
    KillWindow,
    NextWindow,
    PrevWindow,
    LastWindow,
    /// select-window -t <n> (1-based display number)
    SelectWindow(usize),
    RenameWindow(String),
    SelectLayout(LayoutType),
    /// resize-pane -Z
    ToggleZoom,
    /// set synchronize-panes [on|off]; None = toggle
    SetSyncPanes(Option<bool>),
    /// pipe-pane (toggle output logging on the focused pane)
    PipePane,
    /// display-popup [command]
    DisplayPopup { command: Option<String> },
}

/// Parse a prompt line. Returns a user-facing error message on failure.
pub fn parse(line: &str) -> Result<PromptAction, String> {
    let tokens = tokenize(line);
    let Some((name, args)) = tokens.split_first() else {
        return Err("empty command".to_string());
    };
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match name.as_str() {
        "split-window" | "splitw" => {
            let direction = match args.first() {
                // tmux -h splits left/right, which wtmux models as Horizontal
                Some(&"-h") => SplitDirection::Horizontal,
                Some(&"-v") | None => SplitDirection::Vertical,
                Some(other) => return Err(format!("unsupported option: {other}")),
            };
            Ok(PromptAction::Split { direction })
        }
        "new-window" | "neww" => Ok(PromptAction::NewWindow),
        "kill-pane" | "killp" => Ok(PromptAction::KillPane),
        "kill-window" | "killw" => Ok(PromptAction::KillWindow),
        "next-window" | "next" => Ok(PromptAction::NextWindow),
        "previous-window" | "prev" => Ok(PromptAction::PrevWindow),
        "last-window" | "last" => Ok(PromptAction::LastWindow),
        "select-window" | "selectw" => {
            let n = option_value(&args, "-t")
                .ok_or("usage: select-window -t <number>")?
                .parse::<usize>()
                .map_err(|_| "invalid window number".to_string())?;
            Ok(PromptAction::SelectWindow(n))
        }
        "rename-window" | "renamew" => {
            if args.is_empty() {
                return Err("usage: rename-window <name>".to_string());
            }
            Ok(PromptAction::RenameWindow(args.join(" ")))
        }
        "select-layout" | "selectl" => {
            let name = args.first().ok_or(
                "usage: select-layout <even-horizontal|even-vertical|main-horizontal|main-vertical|tiled>",
            )?;
            let layout = LayoutType::parse(name).ok_or_else(|| format!("unknown layout: {name}"))?;
            Ok(PromptAction::SelectLayout(layout))
        }
        "resize-pane" | "resizep" => match args.first() {
            Some(&"-Z") => Ok(PromptAction::ToggleZoom),
            _ => Err("only resize-pane -Z (zoom toggle) is supported".to_string()),
        },
        "set" | "set-option" | "setw" | "set-window-option" => {
            // Accept `set [-w] synchronize-panes [on|off]`
            let args: Vec<&str> = args.iter().copied().filter(|a| *a != "-w").collect();
            match args.as_slice() {
                ["synchronize-panes"] => Ok(PromptAction::SetSyncPanes(None)),
                ["synchronize-panes", "on"] => Ok(PromptAction::SetSyncPanes(Some(true))),
                ["synchronize-panes", "off"] => Ok(PromptAction::SetSyncPanes(Some(false))),
                _ => Err("only `set synchronize-panes [on|off]` is supported".to_string()),
            }
        }
        "pipe-pane" | "pipep" => Ok(PromptAction::PipePane),
        "display-popup" | "popup" => {
            let rest: Vec<&str> = args.iter().copied().filter(|a| *a != "-E").collect();
            let command = if rest.is_empty() {
                None
            } else {
                Some(rest.join(" "))
            };
            Ok(PromptAction::DisplayPopup { command })
        }
        other => Err(format!("unknown command: {other}")),
    }
}

fn option_value<'a>(args: &[&'a str], option: &str) -> Option<&'a str> {
    args.iter()
        .position(|a| *a == option)
        .and_then(|i| args.get(i + 1))
        .copied()
}

/// Split a line into whitespace-separated tokens with single/double quote
/// support (`rename-window "my window"`).
fn tokenize(line: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut has_token = false;

    for ch in line.chars() {
        match quote {
            Some(q) if ch == q => quote = None,
            Some(_) => current.push(ch),
            None if ch == '\'' || ch == '"' => {
                quote = Some(ch);
                has_token = true;
            }
            None if ch.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            None => {
                current.push(ch);
                has_token = true;
            }
        }
    }
    if has_token {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_split_directions_with_tmux_semantics() {
        assert_eq!(
            parse("split-window -h").unwrap(),
            PromptAction::Split {
                direction: SplitDirection::Horizontal
            }
        );
        assert_eq!(
            parse("splitw").unwrap(),
            PromptAction::Split {
                direction: SplitDirection::Vertical
            }
        );
    }

    #[test]
    fn parses_window_commands_and_abbreviations() {
        assert_eq!(parse("new-window").unwrap(), PromptAction::NewWindow);
        assert_eq!(parse("neww").unwrap(), PromptAction::NewWindow);
        assert_eq!(parse("killp").unwrap(), PromptAction::KillPane);
        assert_eq!(parse("kill-window").unwrap(), PromptAction::KillWindow);
        assert_eq!(parse("next").unwrap(), PromptAction::NextWindow);
        assert_eq!(parse("prev").unwrap(), PromptAction::PrevWindow);
        assert_eq!(parse("last").unwrap(), PromptAction::LastWindow);
        assert_eq!(
            parse("select-window -t 3").unwrap(),
            PromptAction::SelectWindow(3)
        );
    }

    #[test]
    fn parses_rename_with_quotes() {
        assert_eq!(
            parse("rename-window \"my window\"").unwrap(),
            PromptAction::RenameWindow("my window".to_string())
        );
        assert_eq!(
            parse("renamew ビルド").unwrap(),
            PromptAction::RenameWindow("ビルド".to_string())
        );
    }

    #[test]
    fn parses_layout_zoom_and_sync() {
        assert_eq!(
            parse("select-layout tiled").unwrap(),
            PromptAction::SelectLayout(LayoutType::Tiled)
        );
        assert!(parse("select-layout bogus").is_err());
        assert_eq!(parse("resize-pane -Z").unwrap(), PromptAction::ToggleZoom);
        assert_eq!(
            parse("set synchronize-panes on").unwrap(),
            PromptAction::SetSyncPanes(Some(true))
        );
        assert_eq!(
            parse("setw -w synchronize-panes").unwrap(),
            PromptAction::SetSyncPanes(None)
        );
    }

    #[test]
    fn parses_popup_and_pipe() {
        assert_eq!(parse("pipe-pane").unwrap(), PromptAction::PipePane);
        assert_eq!(
            parse("display-popup").unwrap(),
            PromptAction::DisplayPopup { command: None }
        );
        assert_eq!(
            parse("popup -E cargo test").unwrap(),
            PromptAction::DisplayPopup {
                command: Some("cargo test".to_string())
            }
        );
    }

    #[test]
    fn rejects_unknown_and_empty_commands() {
        assert!(parse("").is_err());
        assert!(parse("frobnicate").is_err());
    }
}
