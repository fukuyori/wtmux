//! Narrow tmux-compatible CLI surface used by external tools.
//!
//! This is intentionally small: it supports the cwd-sync commands used by
//! tools that already know how to ask tmux for `#{pane_current_path}`.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::core::session::Session;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneSnapshot {
    pub pid: u32,
    pub session_id: String,
    pub window_index: usize,
    pub pane_index: usize,
    pub pane_id: u64,
    pub pane_current_path: String,
    pub pane_title: String,
    pub pane_dead: bool,
    pub pane_width: u16,
    pub pane_height: u16,
}

impl PaneSnapshot {
    pub fn from_session(session: &Session) -> Self {
        Self {
            pid: std::process::id(),
            session_id: session_id_for_pid(std::process::id()),
            window_index: 0,
            pane_index: 0,
            pane_id: session.id,
            pane_current_path: session.state.current_path.clone(),
            pane_title: session.state.title.clone(),
            pane_dead: !session.is_running(),
            pane_width: session.state.cols,
            pane_height: session.state.rows,
        }
    }

    fn to_tsv(&self) -> String {
        let fields = [
            ("version", "1".to_string()),
            ("pid", self.pid.to_string()),
            ("session_id", self.session_id.clone()),
            ("window_index", self.window_index.to_string()),
            ("pane_index", self.pane_index.to_string()),
            ("pane_id", self.pane_id.to_string()),
            ("pane_current_path", self.pane_current_path.clone()),
            ("pane_title", self.pane_title.clone()),
            (
                "pane_dead",
                if self.pane_dead { "1" } else { "0" }.to_string(),
            ),
            ("pane_width", self.pane_width.to_string()),
            ("pane_height", self.pane_height.to_string()),
        ];

        let mut out = String::new();
        for (key, value) in fields {
            out.push_str(key);
            out.push('\t');
            out.push_str(&escape_field(&value));
            out.push('\n');
        }
        out
    }

    fn from_tsv(text: &str) -> Option<Self> {
        let mut map = HashMap::new();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('\t') else {
                continue;
            };
            map.insert(key.to_string(), unescape_field(value));
        }

        let pid = map.get("pid")?.parse().ok()?;
        Some(Self {
            pid,
            session_id: map
                .get("session_id")
                .cloned()
                .unwrap_or_else(|| session_id_for_pid(pid)),
            window_index: map
                .get("window_index")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            pane_index: map
                .get("pane_index")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            pane_id: map.get("pane_id").and_then(|v| v.parse().ok()).unwrap_or(1),
            pane_current_path: map.get("pane_current_path").cloned().unwrap_or_default(),
            pane_title: map.get("pane_title").cloned().unwrap_or_default(),
            pane_dead: map.get("pane_dead").is_some_and(|v| v == "1"),
            pane_width: map
                .get("pane_width")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
            pane_height: map
                .get("pane_height")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        })
    }
}

pub struct StatusPublisher {
    last_text: String,
    last_publish: Instant,
}

impl Default for StatusPublisher {
    fn default() -> Self {
        Self {
            last_text: String::new(),
            last_publish: Instant::now() - HEARTBEAT_INTERVAL,
        }
    }
}

impl StatusPublisher {
    pub fn publish(&mut self, snapshot: &PaneSnapshot) {
        let text = snapshot.to_tsv();
        if text == self.last_text && self.last_publish.elapsed() < HEARTBEAT_INTERVAL {
            return;
        }

        if write_snapshot(snapshot.pid, &text).is_ok() {
            self.last_text = text;
            self.last_publish = Instant::now();
        }
    }
}

pub fn maybe_run_tmux_compat_cli(args: &[String]) -> anyhow::Result<bool> {
    let Some(command) = args.get(1).map(String::as_str) else {
        return Ok(false);
    };

    match command {
        "display-message" => {
            run_display_message(&args[2..])?;
            Ok(true)
        }
        "list-clients" => {
            run_list_clients(&args[2..])?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn session_id_for_pid(pid: u32) -> String {
    format!("${pid}")
}

const HEARTBEAT_INTERVAL: Duration = Duration::from_millis(750);
const STALE_AFTER: Duration = Duration::from_secs(30);

fn run_display_message(args: &[String]) -> anyhow::Result<()> {
    let mut print = false;
    let mut target: Option<String> = None;
    let mut format: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-p" => {
                print = true;
            }
            "-t" => {
                i += 1;
                if i >= args.len() {
                    anyhow::bail!("missing target for -t");
                }
                target = Some(args[i].clone());
            }
            arg if arg.starts_with('-') => {
                anyhow::bail!("unsupported display-message option: {arg}");
            }
            _ => {
                format = Some(args[i].clone());
            }
        }
        i += 1;
    }

    if !print {
        anyhow::bail!("only display-message -p is supported");
    }

    let snapshot = find_snapshot(target.as_deref())?;
    let rendered = expand_format(
        format.as_deref().unwrap_or("#{pane_current_path}"),
        &snapshot,
    );
    println!("{rendered}");
    Ok(())
}

fn run_list_clients(args: &[String]) -> anyhow::Result<()> {
    let mut format = "#{client_pid}\t#{session_id}".to_string();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-F" => {
                i += 1;
                if i >= args.len() {
                    anyhow::bail!("missing format for -F");
                }
                format = args[i].clone();
            }
            arg if arg.starts_with('-') => {
                anyhow::bail!("unsupported list-clients option: {arg}");
            }
            _ => {}
        }
        i += 1;
    }

    for snapshot in read_snapshots()? {
        println!("{}", expand_format(&format, &snapshot));
    }

    Ok(())
}

fn expand_format(format: &str, snapshot: &PaneSnapshot) -> String {
    let mut out = String::with_capacity(format.len());
    let mut rest = format;

    while let Some(start) = rest.find("#{") {
        push_format_literal(&mut out, &rest[..start]);
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find('}') else {
            push_format_literal(&mut out, &rest[start..]);
            return out;
        };

        let name = &after_start[..end];
        out.push_str(&format_value(name, snapshot));
        rest = &after_start[end + 1..];
    }

    push_format_literal(&mut out, rest);
    out
}

fn push_format_literal(out: &mut String, literal: &str) {
    let mut chars = literal.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
}

fn format_value(name: &str, snapshot: &PaneSnapshot) -> String {
    match name {
        "client_pid" => snapshot.pid.to_string(),
        "pane_current_path" => snapshot.pane_current_path.clone(),
        "pane_title" => snapshot.pane_title.clone(),
        "session_id" => snapshot.session_id.clone(),
        "window_index" => snapshot.window_index.to_string(),
        "pane_index" => snapshot.pane_index.to_string(),
        "pane_id" => snapshot.pane_id.to_string(),
        "pane_dead" => {
            if snapshot.pane_dead {
                "1".to_string()
            } else {
                "0".to_string()
            }
        }
        "pane_width" => snapshot.pane_width.to_string(),
        "pane_height" => snapshot.pane_height.to_string(),
        _ => String::new(),
    }
}

fn find_snapshot(target: Option<&str>) -> anyhow::Result<PaneSnapshot> {
    let snapshots = read_snapshots()?;
    if let Some(target) = target {
        let Some(snapshot) = snapshots
            .into_iter()
            .find(|s| s.session_id == target || s.pid.to_string() == target)
        else {
            anyhow::bail!("target not found: {target}");
        };
        return Ok(snapshot);
    }

    snapshots
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("no wtmux session status is available"))
}

fn read_snapshots() -> anyhow::Result<Vec<PaneSnapshot>> {
    let dir = sessions_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };

    let mut snapshots = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("tsv") {
            continue;
        }

        if is_stale(&path)? {
            continue;
        }

        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(snapshot) = PaneSnapshot::from_tsv(&text) {
            snapshots.push((entry.metadata().and_then(|m| m.modified()).ok(), snapshot));
        }
    }

    snapshots.sort_by(|a, b| b.0.cmp(&a.0));
    Ok(snapshots
        .into_iter()
        .map(|(_, snapshot)| snapshot)
        .collect())
}

fn is_stale(path: &Path) -> io::Result<bool> {
    let modified = path.metadata()?.modified()?;
    Ok(SystemTime::now()
        .duration_since(modified)
        .unwrap_or_default()
        > STALE_AFTER)
}

fn write_snapshot(pid: u32, text: &str) -> io::Result<()> {
    let dir = sessions_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{pid}.tsv"));
    let tmp = dir.join(format!("{pid}.tmp"));
    fs::write(&tmp, text)?;
    fs::rename(tmp, path)
}

fn sessions_dir() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("wtmux").join("sessions");
    }

    std::env::temp_dir().join("wtmux").join("sessions")
}

fn escape_field(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn unescape_field(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> PaneSnapshot {
        PaneSnapshot {
            pid: 1234,
            session_id: "$1234".to_string(),
            window_index: 2,
            pane_index: 3,
            pane_id: 9,
            pane_current_path: "C:\\tmp\\demo".to_string(),
            pane_title: "demo".to_string(),
            pane_dead: false,
            pane_width: 80,
            pane_height: 24,
        }
    }

    #[test]
    fn expands_tmux_format_subset() {
        let snapshot = sample();

        assert_eq!(
            expand_format(
                "#{client_pid}\t#{session_id}\t#{pane_current_path}\t#{pane_dead}",
                &snapshot,
            ),
            "1234\t$1234\tC:\\tmp\\demo\t0"
        );
    }

    #[test]
    fn expands_backslash_t_in_format_literals_only() {
        let snapshot = sample();

        assert_eq!(
            expand_format("#{client_pid}\\t#{pane_current_path}", &snapshot),
            "1234\tC:\\tmp\\demo"
        );
    }

    #[test]
    fn snapshot_round_trips_tsv() {
        let snapshot = sample();
        let parsed = PaneSnapshot::from_tsv(&snapshot.to_tsv()).unwrap();

        assert_eq!(parsed, snapshot);
    }
}
