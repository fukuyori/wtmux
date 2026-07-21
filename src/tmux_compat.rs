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
        "agents" => {
            run_agents(&args[2..])?;
            Ok(true)
        }
        "report-state" => {
            run_report_state(&args[2..])?;
            Ok(true)
        }
        "send-keys" => {
            run_send_keys(&args[2..])?;
            Ok(true)
        }
        "capture-pane" => {
            run_capture_pane(&args[2..])?;
            Ok(true)
        }
        "display-popup" => {
            run_display_popup(&args[2..])?;
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

/// `wtmux report-state [-t <window>.<pane>] [--pid <pid>] <idle|working|blocked|done>`
///
/// Reports the ground-truth agent state of a pane to the running wtmux
/// instance, overriding its output heuristics. Designed to be called from an
/// agent CLI's own hooks (e.g. Claude Code Stop / Notification hooks): the
/// target defaults to the calling pane via the `WTMUX_PID` / `WTMUX_PANE`
/// environment variables that wtmux sets for every pane's child process.
fn run_report_state(args: &[String]) -> anyhow::Result<()> {
    let mut target: Option<String> = None;
    let mut pid_override: Option<u32> = None;
    let mut state: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-t" => {
                i += 1;
                target = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("missing target for -t"))?
                        .clone(),
                );
            }
            "--pid" => {
                i += 1;
                pid_override = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("missing pid for --pid"))?
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid pid"))?,
                );
            }
            arg if arg.starts_with('-') => {
                anyhow::bail!("unsupported report-state option: {arg}");
            }
            _ => {
                state = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let state = state
        .ok_or_else(|| anyhow::anyhow!("usage: wtmux report-state [-t <window>.<pane>] [--pid <pid>] <idle|working|blocked|done>"))?
        .to_ascii_lowercase();
    if !matches!(state.as_str(), "idle" | "working" | "blocked" | "done") {
        anyhow::bail!("invalid state {state:?}: expected idle, working, blocked, or done");
    }

    let pid = match pid_override {
        Some(pid) => pid,
        None => std::env::var("WTMUX_PID")
            .ok()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| {
                anyhow::anyhow!("not inside a wtmux pane (WTMUX_PID unset); pass --pid <pid>")
            })?,
    };
    let pane = match target {
        Some(t) => t,
        None => std::env::var("WTMUX_PANE").map_err(|_| {
            anyhow::anyhow!("no target pane (WTMUX_PANE unset); pass -t <window>.<pane>")
        })?,
    };
    if !pane
        .split_once('.')
        .is_some_and(|(w, p)| w.parse::<u64>().is_ok() && p.parse::<u64>().is_ok())
    {
        anyhow::bail!("invalid target {pane:?}: expected <window>.<pane>, e.g. 1.2");
    }

    let dir = agent_state_dir(pid);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{pane}.tmp"));
    let path = dir.join(format!("{pane}.tsv"));
    fs::write(&tmp, format!("version\t1\nstate\t{state}\n"))?;
    fs::rename(tmp, path)?;
    Ok(())
}

/// Remove leftover report-state drops and request files for this pid
/// (pid reuse after a previous run) so stale data is not applied to fresh
/// panes.
pub fn cleanup_agent_state_dir() {
    let _ = fs::remove_dir_all(agent_state_dir(std::process::id()));
    let _ = fs::remove_dir_all(requests_dir(std::process::id()));
}

// ─── Request/reply IPC (send-keys / capture-pane / display-popup) ──────────
//
// The CLI writes `<id>.req.tsv` into the target instance's requests dir and
// polls for `<id>.rep.tsv`; the instance's main loop drains requests, acts on
// them, and writes the reply. Both files are removed by their consumer.

/// A request from an external `wtmux` CLI invocation to a running instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatRequest {
    pub id: String,
    pub command: String,
    pub target: Option<String>,
    pub args: Vec<String>,
}

fn requests_dir(pid: u32) -> PathBuf {
    runtime_root().join("requests").join(pid.to_string())
}

/// How long the CLI waits for the running instance to answer a request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
/// CLI-side reply poll interval.
const REQUEST_POLL: Duration = Duration::from_millis(20);

/// Resolve which wtmux instance a CLI command targets: `--pid` beats the
/// `WTMUX_PID` env var; failing both, a single live instance is used.
fn resolve_instance_pid(pid_override: Option<u32>) -> anyhow::Result<u32> {
    if let Some(pid) = pid_override {
        return Ok(pid);
    }
    if let Some(pid) = std::env::var("WTMUX_PID")
        .ok()
        .and_then(|v| v.parse().ok())
    {
        return Ok(pid);
    }

    let mut pids: Vec<u32> = read_snapshots()?.into_iter().map(|s| s.pid).collect();
    pids.dedup();
    match pids.as_slice() {
        [pid] => Ok(*pid),
        [] => anyhow::bail!("no running wtmux instance found; pass --pid <pid>"),
        many => anyhow::bail!(
            "multiple wtmux instances running ({}); pass --pid <pid>",
            many.iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// Send a request to the instance and wait for its reply payload.
fn send_request(pid: u32, command: &str, target: Option<&str>, args: &[String]) -> anyhow::Result<Option<String>> {
    let id = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    let mut text = String::new();
    text.push_str(&format!("command\t{}\n", escape_field(command)));
    if let Some(target) = target {
        text.push_str(&format!("target\t{}\n", escape_field(target)));
    }
    for arg in args {
        text.push_str(&format!("arg\t{}\n", escape_field(arg)));
    }

    let dir = requests_dir(pid);
    fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!("{id}.tmp"));
    let req = dir.join(format!("{id}.req.tsv"));
    let rep = dir.join(format!("{id}.rep.tsv"));
    fs::write(&tmp, text)?;
    fs::rename(&tmp, &req)?;

    let deadline = Instant::now() + REQUEST_TIMEOUT;
    loop {
        if let Ok(text) = fs::read_to_string(&rep) {
            let _ = fs::remove_file(&rep);
            let mut error: Option<String> = None;
            let mut data: Option<String> = None;
            for line in text.lines() {
                if let Some(v) = line.strip_prefix("error\t") {
                    error = Some(unescape_field(v));
                } else if let Some(v) = line.strip_prefix("data\t") {
                    data = Some(unescape_field(v));
                }
            }
            if let Some(error) = error {
                anyhow::bail!("{error}");
            }
            return Ok(data);
        }
        if Instant::now() >= deadline {
            let _ = fs::remove_file(&req);
            anyhow::bail!("no response from wtmux (pid {pid}); is it running?");
        }
        std::thread::sleep(REQUEST_POLL);
    }
}

/// Instance side: consume pending request files for this process.
pub fn drain_requests() -> Vec<CompatRequest> {
    let dir = requests_dir(std::process::id());
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut with_time: Vec<(SystemTime, CompatRequest)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(id) = name.strip_suffix(".req.tsv") else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let _ = fs::remove_file(&path);

        let mut command = String::new();
        let mut target = None;
        let mut args = Vec::new();
        for line in text.lines() {
            if let Some(v) = line.strip_prefix("command\t") {
                command = unescape_field(v);
            } else if let Some(v) = line.strip_prefix("target\t") {
                target = Some(unescape_field(v));
            } else if let Some(v) = line.strip_prefix("arg\t") {
                args.push(unescape_field(v));
            }
        }
        if command.is_empty() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        with_time.push((
            modified,
            CompatRequest {
                id: id.to_string(),
                command,
                target,
                args,
            },
        ));
    }

    with_time.sort_by(|a, b| a.0.cmp(&b.0));
    with_time.into_iter().map(|(_, req)| req).collect()
}

/// Instance side: answer a drained request.
pub fn write_reply(id: &str, result: &Result<Option<String>, String>) {
    let dir = requests_dir(std::process::id());
    let mut text = String::new();
    match result {
        Ok(data) => {
            text.push_str("ok\t1\n");
            if let Some(data) = data {
                text.push_str(&format!("data\t{}\n", escape_field(data)));
            }
        }
        Err(error) => {
            text.push_str(&format!("error\t{}\n", escape_field(error)));
        }
    }

    let tmp = dir.join(format!("{id}.rep.tmp"));
    let rep = dir.join(format!("{id}.rep.tsv"));
    if fs::create_dir_all(&dir).is_ok() && fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &rep);
    }
}

/// Translate `send-keys` arguments to the byte stream written to the pane.
/// Each argument is either a recognized key name (Enter, Escape, Tab, Space,
/// BSpace, arrows, C-x, M-x, ...) or literal text.
pub fn send_keys_to_bytes(args: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for arg in args {
        match key_name_to_bytes(arg) {
            Some(bytes) => out.extend_from_slice(&bytes),
            None => out.extend_from_slice(arg.as_bytes()),
        }
    }
    out
}

fn key_name_to_bytes(name: &str) -> Option<Vec<u8>> {
    // C-x / ^X control keys
    if let Some(rest) = name.strip_prefix("C-").or_else(|| name.strip_prefix("^")) {
        let mut chars = rest.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if ch.is_ascii_alphabetic() {
                return Some(vec![(ch.to_ascii_lowercase() as u8) - b'a' + 1]);
            }
        }
        return None;
    }
    // M-x meta keys
    if let Some(rest) = name.strip_prefix("M-") {
        let mut chars = rest.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            return Some(vec![0x1B, ch as u8]);
        }
        return None;
    }

    match name {
        "Enter" | "KPEnter" => Some(b"\r".to_vec()),
        "Tab" => Some(b"\t".to_vec()),
        "BTab" => Some(b"\x1b[Z".to_vec()),
        "Space" => Some(b" ".to_vec()),
        "Escape" | "Esc" => Some(b"\x1b".to_vec()),
        "BSpace" | "Backspace" => Some(b"\x7f".to_vec()),
        "Up" => Some(b"\x1b[A".to_vec()),
        "Down" => Some(b"\x1b[B".to_vec()),
        "Right" => Some(b"\x1b[C".to_vec()),
        "Left" => Some(b"\x1b[D".to_vec()),
        "Home" => Some(b"\x1b[H".to_vec()),
        "End" => Some(b"\x1b[F".to_vec()),
        "PageUp" | "PgUp" => Some(b"\x1b[5~".to_vec()),
        "PageDown" | "PgDn" => Some(b"\x1b[6~".to_vec()),
        "Delete" | "DC" => Some(b"\x1b[3~".to_vec()),
        _ => None,
    }
}

/// `wtmux send-keys [-t <window>.<pane>] [--pid <pid>] <keys...>`
fn run_send_keys(args: &[String]) -> anyhow::Result<()> {
    let (target, pid_override, rest) = split_common_options(args)?;
    if rest.is_empty() {
        anyhow::bail!("usage: wtmux send-keys [-t <window>.<pane>] [--pid <pid>] <keys...>");
    }
    let pid = resolve_instance_pid(pid_override)?;
    let target = target.or_else(|| std::env::var("WTMUX_PANE").ok());
    send_request(pid, "send-keys", target.as_deref(), &rest)?;
    Ok(())
}

/// `wtmux capture-pane -p [-t <window>.<pane>] [--pid <pid>] [-S -]`
fn run_capture_pane(args: &[String]) -> anyhow::Result<()> {
    let mut print = false;
    let mut scrollback = false;
    let mut filtered = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" => print = true,
            "-S" => {
                i += 1;
                match args.get(i).map(String::as_str) {
                    Some("-") => scrollback = true,
                    other => anyhow::bail!(
                        "unsupported -S value {:?}: only \"-\" (full scrollback) is supported",
                        other.unwrap_or("")
                    ),
                }
            }
            _ => filtered.push(args[i].clone()),
        }
        i += 1;
    }
    if !print {
        anyhow::bail!("only capture-pane -p (print to stdout) is supported");
    }

    let (target, pid_override, rest) = split_common_options(&filtered)?;
    if !rest.is_empty() {
        anyhow::bail!("unexpected argument: {}", rest[0]);
    }
    let pid = resolve_instance_pid(pid_override)?;
    let target = target.or_else(|| std::env::var("WTMUX_PANE").ok());
    let args: Vec<String> = if scrollback {
        vec!["scrollback".to_string()]
    } else {
        Vec::new()
    };
    let data = send_request(pid, "capture-pane", target.as_deref(), &args)?;
    println!("{}", data.unwrap_or_default());
    Ok(())
}

/// `wtmux display-popup [-E] [--pid <pid>] [command...]`
///
/// Without `-E` a command popup stays open after the command exits (tmux
/// semantics); `-E` closes it automatically.
fn run_display_popup(args: &[String]) -> anyhow::Result<()> {
    let mut auto_close = false;
    let mut filtered = Vec::new();
    for arg in args {
        if arg == "-E" {
            auto_close = true;
            continue;
        }
        filtered.push(arg.clone());
    }
    let (_, pid_override, rest) = split_common_options(&filtered)?;
    let pid = resolve_instance_pid(pid_override)?;
    let mut command = Vec::new();
    if auto_close {
        command.push("-E".to_string());
    }
    if !rest.is_empty() {
        command.push(rest.join(" "));
    }
    send_request(pid, "display-popup", None, &command)?;
    Ok(())
}

/// One row of the `list-agents` IPC reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLine {
    pub window_number: usize,
    pub window_name: String,
    pub pane_number: usize,
    pub pane_title: String,
    pub state: String,
    pub attention: bool,
    pub focused: bool,
}

/// Serialize the agent overview for the `list-agents` reply: one TSV row
/// per pane.
pub fn format_agent_lines(entries: &[crate::wm::AgentEntry]) -> String {
    let mut out = String::new();
    for entry in entries {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            entry.window_number,
            escape_field(&entry.window_name),
            entry.pane_number,
            escape_field(&entry.pane_title),
            entry.state.label(),
            u8::from(entry.attention),
            u8::from(entry.is_focused),
        ));
    }
    out
}

fn parse_agent_lines(text: &str) -> Vec<AgentLine> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() != 7 {
                return None;
            }
            Some(AgentLine {
                window_number: fields[0].parse().ok()?,
                window_name: unescape_field(fields[1]),
                pane_number: fields[2].parse().ok()?,
                pane_title: unescape_field(fields[3]),
                state: fields[4].to_string(),
                attention: fields[5] == "1",
                focused: fields[6] == "1",
            })
        })
        .collect()
}

/// One dashboard-style row with ANSI state colors; WORKING rows animate a
/// Nerd Font spinner:
/// ` * 1:main · 2: claude    󰪡 WORKING`
fn render_agent_line(line: &AgentLine, tick: usize) -> String {
    let color = match line.state.as_str() {
        "WORKING" => "\x1b[32m",
        "BLOCKED" => "\x1b[33m",
        "DONE" => "\x1b[36m",
        _ => "\x1b[90m",
    };
    let focus = if line.focused { '*' } else { ' ' };
    let attention = if line.attention { '!' } else { ' ' };
    let spinner = if line.state == "WORKING" {
        crate::wm::pane::working_spinner_frame(tick)
    } else {
        ' '
    };
    format!(
        " {} {}:{} · {}: {:<24} {}{}{} {:<8}\x1b[0m",
        focus,
        line.window_number,
        line.window_name,
        line.pane_number,
        line.pane_title,
        color,
        attention,
        spinner,
        line.state,
    )
}

/// How often `wtmux agents` refreshes (one spinner frame per refresh).
const AGENTS_REFRESH: Duration =
    Duration::from_millis(crate::wm::pane::WORKING_SPINNER_INTERVAL_MS);

/// `wtmux agents [--once] [--pid <pid>]`
///
/// Herdr-style agent monitor meant to run inside a pane (or a popup): the
/// same WORKING / BLOCKED / DONE / IDLE list as the `Prefix + g` dashboard,
/// refreshed every second until Ctrl+C. `--once` prints once and exits.
fn run_agents(args: &[String]) -> anyhow::Result<()> {
    let mut once = false;
    let mut filtered = Vec::new();
    for arg in args {
        match arg.as_str() {
            "--once" | "-1" => once = true,
            other => filtered.push(other.to_string()),
        }
    }
    let (_, pid_override, rest) = split_common_options(&filtered)?;
    if !rest.is_empty() {
        anyhow::bail!("usage: wtmux agents [--once] [--pid <pid>]");
    }
    let pid = resolve_instance_pid(pid_override)?;

    if !once {
        // Start from a clean screen; afterwards redraw in place
        print!("\x1b[2J");
    }
    let mut tick = 0usize;
    loop {
        let reply = send_request(pid, "list-agents", None, &[])?.unwrap_or_default();
        let lines = parse_agent_lines(&reply);

        if once {
            for line in &lines {
                println!("{}", render_agent_line(line, tick));
            }
            return Ok(());
        }

        // Home the cursor and overwrite each row (\x1b[K erases the tail)
        // instead of clearing the whole screen — avoids flicker.
        let mut out = String::from("\x1b[H");
        out.push_str(&format!(
            "\x1b[1mwtmux agents\x1b[0m — {} pane(s), 1s refresh, Ctrl+C quits\x1b[K\n\x1b[K\n",
            lines.len()
        ));
        for line in &lines {
            out.push_str(&render_agent_line(line, tick));
            out.push_str("\x1b[K\n");
        }
        // Erase anything left below (rows that disappeared)
        out.push_str("\x1b[J");
        print!("{out}");
        io::Write::flush(&mut io::stdout())?;
        tick = tick.wrapping_add(1);
        std::thread::sleep(AGENTS_REFRESH);
    }
}

/// Parse the `-t <target>` / `--pid <pid>` options shared by the IPC
/// commands, returning the remaining positional arguments.
fn split_common_options(args: &[String]) -> anyhow::Result<(Option<String>, Option<u32>, Vec<String>)> {
    let mut target = None;
    let mut pid = None;
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-t" => {
                i += 1;
                target = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("missing target for -t"))?
                        .clone(),
                );
            }
            "--pid" => {
                i += 1;
                pid = Some(
                    args.get(i)
                        .ok_or_else(|| anyhow::anyhow!("missing pid for --pid"))?
                        .parse()
                        .map_err(|_| anyhow::anyhow!("invalid pid"))?,
                );
            }
            _ => rest.push(args[i].clone()),
        }
        i += 1;
    }
    Ok((target, pid, rest))
}

/// Consume every state file reported for this wtmux process, returning
/// `(tab_id, pane_id, state)` triples. Files are deleted after reading so a
/// re-report of the same state fires again.
pub fn drain_reported_states() -> Vec<(u64, u64, crate::wm::pane::AgentState)> {
    let dir = agent_state_dir(std::process::id());
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|v| v.to_str()) != Some("tsv") {
            continue;
        }
        let parsed = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|stem| stem.split_once('.'))
            .and_then(|(tab, pane)| Some((tab.parse::<u64>().ok()?, pane.parse::<u64>().ok()?)));
        let state = fs::read_to_string(&path).ok().and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("state\t"))
                .and_then(crate::wm::pane::AgentState::parse)
        });
        // Consume the file either way so malformed reports don't pile up
        let _ = fs::remove_file(&path);
        if let (Some((tab_id, pane_id)), Some(state)) = (parsed, state) {
            out.push((tab_id, pane_id, state));
        }
    }
    out
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

fn runtime_root() -> PathBuf {
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local_app_data).join("wtmux");
    }

    std::env::temp_dir().join("wtmux")
}

fn sessions_dir() -> PathBuf {
    runtime_root().join("sessions")
}

/// Per-instance drop directory for `wtmux report-state` files.
fn agent_state_dir(pid: u32) -> PathBuf {
    runtime_root().join("agent-state").join(pid.to_string())
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

    fn report_args(pid: u32, target: &str, state: &str) -> Vec<String> {
        ["--pid", &pid.to_string(), "-t", target, state]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn agent_lines_round_trip_with_escaped_fields() {
        use crate::wm::{AgentEntry, AgentState};

        let entries = vec![AgentEntry {
            window_index: 0,
            pane_index: 1,
            window_number: 1,
            window_name: "main\twin".to_string(),
            pane_number: 2,
            pane_title: "claude".to_string(),
            state: AgentState::Blocked,
            attention: true,
            is_focused: false,
        }];
        let text = format_agent_lines(&entries);
        let parsed = parse_agent_lines(&text);
        assert_eq!(parsed.len(), 1);
        let line = &parsed[0];
        assert_eq!(line.window_name, "main\twin");
        assert_eq!(line.pane_title, "claude");
        assert_eq!(line.state, "BLOCKED");
        assert!(line.attention);
        assert!(!line.focused);
    }

    #[test]
    fn report_state_round_trips_through_drop_dir() {
        let pid = std::process::id();
        cleanup_agent_state_dir();

        run_report_state(&report_args(pid, "3.2", "Blocked")).unwrap();
        let drained = drain_reported_states();
        assert!(drained.contains(&(3, 2, crate::wm::pane::AgentState::Blocked)));
        assert!(
            drain_reported_states().is_empty(),
            "reports are consumed on read"
        );

        assert!(run_report_state(&report_args(pid, "3.2", "bogus")).is_err());
        assert!(run_report_state(&report_args(pid, "not-a-pane", "done")).is_err());
        // Missing state argument errors regardless of environment
        assert!(
            run_report_state(&["--pid".into(), pid.to_string(), "-t".into(), "1.1".into()])
                .is_err()
        );
    }

    #[test]
    fn send_keys_args_translate_key_names_and_literals() {
        let args: Vec<String> = ["echo hi", "Enter", "C-c", "Escape", "Up", "M-x", "not-a-key"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            send_keys_to_bytes(&args),
            b"echo hi\r\x03\x1b\x1b[A\x1bxnot-a-key".to_vec()
        );
    }

    #[test]
    fn requests_round_trip_between_cli_and_instance() {
        let pid = std::process::id();
        let _ = fs::remove_dir_all(requests_dir(pid));

        // Instance-side reply happens on another thread, as in real use.
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let requests = drain_requests();
                if let Some(req) = requests.first() {
                    assert_eq!(req.command, "capture-pane");
                    assert_eq!(req.target.as_deref(), Some("1.2"));
                    assert_eq!(req.args, vec!["scrollback".to_string()]);
                    write_reply(&req.id, &Ok(Some("line1\nline2".to_string())));
                    return;
                }
                assert!(Instant::now() < deadline, "request never arrived");
                std::thread::sleep(Duration::from_millis(10));
            }
        });

        let data = send_request(
            pid,
            "capture-pane",
            Some("1.2"),
            &["scrollback".to_string()],
        )
        .unwrap();
        assert_eq!(data.as_deref(), Some("line1\nline2"));
        handle.join().unwrap();

        // Error replies surface as CLI errors
        let handle = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let requests = drain_requests();
                if let Some(req) = requests.first() {
                    write_reply(&req.id, &Err("pane 9.9 not found".to_string()));
                    return;
                }
                assert!(Instant::now() < deadline, "request never arrived");
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let err = send_request(pid, "send-keys", Some("9.9"), &["x".to_string()]).unwrap_err();
        assert!(err.to_string().contains("pane 9.9 not found"));
        handle.join().unwrap();
    }
}
