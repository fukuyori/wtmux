//! POSIX pty backend for macOS and Linux.
//!
//! Mirrors the ConPTY wrapper's API so `Session` can drive either backend
//! unchanged: `read` is non-blocking (returns `Ok(0)` when no data is
//! available), `write` writes the whole buffer, and process liveness is
//! queried through `is_running` / `exit_code`.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum PtyError {
    #[error("Failed to open pty: {0}")]
    Open(#[source] io::Error),

    #[error("Failed to spawn process: {0}")]
    ProcessSpawn(#[source] io::Error),

    #[allow(dead_code)]
    #[error("Failed to resize pty: {0}")]
    Resize(#[source] io::Error),

    #[error("Failed to read from PTY: {0}")]
    Read(#[source] io::Error),

    #[error("Failed to write to PTY: {0}")]
    Write(#[source] io::Error),

    #[allow(dead_code)]
    #[error("Process has exited with code: {0}")]
    ProcessExited(u32),

    #[error("Invalid handle")]
    InvalidHandle,
}

pub type Result<T> = std::result::Result<T, PtyError>;

/// POSIX pty wrapper (master side) with the spawned shell process.
pub struct UnixPty {
    master: OwnedFd,
    /// Mutex because `is_running`/`exit_code` take `&self` (the pty is shared
    /// through an `Arc` with the reader thread) but `Child::try_wait` needs
    /// `&mut Child`.
    child: Mutex<Child>,
    #[allow(dead_code)]
    cols: u16,
    #[allow(dead_code)]
    rows: u16,
}

impl UnixPty {
    /// Create a new pty and spawn a shell.
    #[allow(dead_code)]
    pub fn new(cols: u16, rows: u16, command: Option<&str>) -> Result<Self> {
        Self::new_with_options(cols, rows, command, None, false)
    }

    /// Create a new pty and spawn a shell.
    ///
    /// `codepage` and `cwd_prompt_hook` are Windows shell concepts (chcp /
    /// cmd-and-PowerShell prompt rewriting) and are ignored here; Unix shells
    /// speak UTF-8 and publish their cwd via OSC 7 themselves when configured.
    pub fn new_with_options(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        _codepage: Option<u32>,
        _cwd_prompt_hook: bool,
    ) -> Result<Self> {
        let (master, slave) = open_pty_pair(cols, rows).map_err(PtyError::Open)?;

        let mut cmd = build_shell_command(command);
        let slave_in = slave.try_clone().map_err(PtyError::Open)?;
        let slave_out = slave.try_clone().map_err(PtyError::Open)?;
        cmd.stdin(Stdio::from(slave_in))
            .stdout(Stdio::from(slave_out))
            .stderr(Stdio::from(slave))
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor");

        unsafe {
            cmd.pre_exec(|| {
                // Make the child a session leader with the pty slave (now on
                // fd 0) as its controlling terminal, so job control and
                // SIGHUP-on-close work like in a real terminal.
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = cmd.spawn().map_err(PtyError::ProcessSpawn)?;

        Ok(UnixPty {
            master,
            child: Mutex::new(child),
            cols,
            rows,
        })
    }

    /// Resize the pty.
    #[allow(dead_code)]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        self.resize_pty(cols, rows)?;
        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Resize the pty (immutable version for use with Arc).
    pub fn resize_pty(&self, cols: u16, rows: u16) -> Result<()> {
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let rc = unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ as _, &ws) };
        if rc != 0 {
            return Err(PtyError::Resize(io::Error::last_os_error()));
        }
        Ok(())
    }

    /// Write bytes to the pty (input to shell). Writes the whole buffer.
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        let fd = self.master.as_raw_fd();
        let mut written = 0;
        while written < data.len() {
            let n = unsafe {
                libc::write(
                    fd,
                    data[written..].as_ptr() as *const libc::c_void,
                    data.len() - written,
                )
            };
            if n < 0 {
                let err = io::Error::last_os_error();
                match err.raw_os_error() {
                    // The master is O_NONBLOCK; back off briefly if the
                    // kernel input buffer is full (large pastes).
                    Some(libc::EAGAIN) => std::thread::sleep(Duration::from_millis(1)),
                    Some(libc::EINTR) => {}
                    _ => return Err(PtyError::Write(err)),
                }
            } else {
                written += n as usize;
            }
        }
        Ok(written)
    }

    /// Read bytes from the pty (output from shell) — non-blocking.
    /// Returns `Ok(0)` when no data is currently available.
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize> {
        let n = unsafe {
            libc::read(
                self.master.as_raw_fd(),
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => Ok(0),
                // Linux reports EIO on the master once the slave side is
                // gone; treat it like EOF.
                _ => Err(PtyError::Read(err)),
            }
        } else if n == 0 {
            // EOF: the shell exited and the slave side closed.
            Err(PtyError::Read(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "pty closed",
            )))
        } else {
            Ok(n as usize)
        }
    }

    /// Check if the process is still running.
    pub fn is_running(&self) -> bool {
        match self.child.lock() {
            Ok(mut child) => matches!(child.try_wait(), Ok(None)),
            Err(_) => false,
        }
    }

    /// Get the exit code if the process has exited.
    #[allow(dead_code)]
    pub fn exit_code(&self) -> Option<u32> {
        let mut child = self.child.lock().ok()?;
        match child.try_wait() {
            Ok(Some(status)) => Some(status.code().unwrap_or(-1) as u32),
            _ => None,
        }
    }

    /// Get current size.
    #[allow(dead_code)]
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Cancel pending read operations. Reads are non-blocking on Unix and the
    /// reader thread polls its stop flag, so nothing to do here.
    pub fn cancel_read(&self) {}
}

impl Drop for UnixPty {
    fn drop(&mut self) {
        let Ok(mut child) = self.child.lock() else {
            return;
        };
        if matches!(child.try_wait(), Ok(None)) {
            // Ask the shell to hang up first (what closing a terminal does),
            // then force-kill if it doesn't exit promptly. Always reap so no
            // zombie is left behind.
            unsafe {
                libc::kill(child.id() as libc::pid_t, libc::SIGHUP);
            }
            for _ in 0..50 {
                if !matches!(child.try_wait(), Ok(None)) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Open a master/slave pty pair sized to `cols` x `rows`.
///
/// The master is set non-blocking (the `read` contract above) and
/// close-on-exec so it doesn't leak into the spawned shell.
fn open_pty_pair(cols: u16, rows: u16) -> io::Result<(OwnedFd, OwnedFd)> {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let mut ws = libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    let rc = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut ws,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }

    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };

    unsafe {
        let fd = master.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 || libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok((master, slave))
}

/// Build the shell command to spawn.
///
/// With no explicit command, `$SHELL` (falling back to `/bin/sh`) is spawned
/// directly. A command string containing whitespace is run through
/// `/bin/sh -c` so pipelines and arguments work.
fn build_shell_command(command: Option<&str>) -> Command {
    let cmd_str = command
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(default_shell);

    if cmd_str.split_whitespace().count() > 1 {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(cmd_str);
        cmd
    } else {
        Command::new(cmd_str)
    }
}

fn default_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all_with_timeout(pty: &UnixPty, timeout: Duration) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        let start = std::time::Instant::now();
        loop {
            match pty.read(&mut buf) {
                Ok(0) => {
                    if !pty.is_running() && start.elapsed() > Duration::from_millis(200) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
            if start.elapsed() > timeout {
                break;
            }
        }
        out
    }

    #[test]
    fn spawns_command_and_captures_output() {
        let pty = UnixPty::new(80, 24, Some("echo hello-wtmux")).expect("spawn pty");
        let out = read_all_with_timeout(&pty, Duration::from_secs(10));
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("hello-wtmux"), "output was: {text:?}");
    }

    #[test]
    fn write_reaches_the_shell() {
        let pty = UnixPty::new(80, 24, Some("/bin/cat")).expect("spawn pty");
        pty.write(b"ping\n").expect("write");
        let out = read_all_with_timeout(&pty, Duration::from_secs(10));
        let text = String::from_utf8_lossy(&out);
        assert!(text.contains("ping"), "output was: {text:?}");
    }

    #[test]
    fn exit_code_is_reported() {
        let pty = UnixPty::new(80, 24, Some("sh -c 'exit 3'")).expect("spawn pty");
        let start = std::time::Instant::now();
        while pty.is_running() && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(pty.exit_code(), Some(3));
    }

    #[test]
    fn default_shell_falls_back_to_sh() {
        assert!(!default_shell().is_empty());
    }
}
