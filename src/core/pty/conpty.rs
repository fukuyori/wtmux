//! ConPTY wrapper for Windows
//!
//! This module provides a safe wrapper around Windows ConPTY (Console Pseudo Terminal)
//! for creating and managing pseudo-terminal sessions.

use std::io;
use thiserror::Error;

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows::Win32::System::Pipes::{CreatePipe, PeekNamedPipe};
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, GetExitCodeProcess,
    InitializeProcThreadAttributeList, UpdateProcThreadAttribute, WaitForSingleObject,
    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    STARTUPINFOEXW,
};
use windows::Win32::System::IO::CancelIoEx;
use windows::core::{PCWSTR, PWSTR};

#[derive(Error, Debug)]
pub enum PtyError {
    #[error("Failed to create pipe: {0}")]
    PipeCreation(#[source] windows::core::Error),

    #[error("Failed to create pseudo console: {0}")]
    ConPtyCreation(#[source] windows::core::Error),

    #[error("Failed to spawn process: {0}")]
    ProcessSpawn(#[source] windows::core::Error),

    #[allow(dead_code)]
    #[error("Failed to resize pseudo console: {0}")]
    Resize(#[source] windows::core::Error),

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

/// ConPTY handle wrapper
pub struct ConPty {
    hpc: HPCON,
    input_write: HANDLE,
    output_read: HANDLE,
    process: PROCESS_INFORMATION,
    #[allow(dead_code)]
    cols: u16,
    #[allow(dead_code)]
    rows: u16,
}

// Safety: ConPty handles are thread-safe when accessed properly
unsafe impl Send for ConPty {}

/// RAII guard that closes a Win32 HANDLE on drop unless released.
/// Ensures early returns in `create_internal` don't leak pipe handles.
struct HandleGuard(Option<HANDLE>);

impl HandleGuard {
    fn new(handle: HANDLE) -> Self {
        Self(Some(handle))
    }

    fn get(&self) -> HANDLE {
        self.0.unwrap_or_default()
    }

    /// Take ownership of the handle; the guard will no longer close it
    fn release(mut self) -> HANDLE {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            if !handle.is_invalid() {
                unsafe {
                    let _ = CloseHandle(handle);
                }
            }
        }
    }
}

/// RAII guard that closes a pseudo console on drop unless released
struct HpcGuard(Option<HPCON>);

impl HpcGuard {
    fn get(&self) -> HPCON {
        self.0.unwrap_or_default()
    }

    fn release(mut self) -> HPCON {
        self.0.take().unwrap_or_default()
    }
}

impl Drop for HpcGuard {
    fn drop(&mut self) {
        if let Some(hpc) = self.0.take() {
            unsafe { ClosePseudoConsole(hpc) };
        }
    }
}

/// RAII guard that deletes a proc-thread attribute list on drop
struct AttrListGuard(LPPROC_THREAD_ATTRIBUTE_LIST);

impl Drop for AttrListGuard {
    fn drop(&mut self) {
        unsafe { DeleteProcThreadAttributeList(self.0) };
    }
}

impl ConPty {
    /// Create a new ConPTY instance and spawn a shell
    #[allow(dead_code)]
    pub fn new(cols: u16, rows: u16, command: Option<&str>) -> Result<Self> {
        unsafe { Self::create_internal(cols, rows, command, None, false) }
    }

    /// Create a new ConPTY instance with specific codepage
    #[allow(dead_code)]
    pub fn new_with_codepage(cols: u16, rows: u16, command: Option<&str>, codepage: Option<u32>) -> Result<Self> {
        Self::new_with_options(cols, rows, command, codepage, false)
    }

    /// Create a new ConPTY instance with specific codepage and shell options.
    pub fn new_with_options(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        codepage: Option<u32>,
        cwd_prompt_hook: bool,
    ) -> Result<Self> {
        unsafe { Self::create_internal(cols, rows, command, codepage, cwd_prompt_hook) }
    }

    unsafe fn create_internal(
        cols: u16,
        rows: u16,
        command: Option<&str>,
        codepage: Option<u32>,
        cwd_prompt_hook: bool,
    ) -> Result<Self> {
        // Create pipes for PTY communication
        let mut pty_input_read = HANDLE::default();
        let mut pty_input_write = HANDLE::default();
        let mut pty_output_read = HANDLE::default();
        let mut pty_output_write = HANDLE::default();

        // Input pipe (we write, PTY reads).  Guards close the handles on any
        // early return below so a failed spawn doesn't leak them.
        CreatePipe(&mut pty_input_read, &mut pty_input_write, None, 0)
            .map_err(PtyError::PipeCreation)?;
        let pty_input_read = HandleGuard::new(pty_input_read);
        let pty_input_write = HandleGuard::new(pty_input_write);

        // Output pipe (PTY writes, we read)
        CreatePipe(&mut pty_output_read, &mut pty_output_write, None, 0)
            .map_err(PtyError::PipeCreation)?;
        let pty_output_read = HandleGuard::new(pty_output_read);
        let pty_output_write = HandleGuard::new(pty_output_write);

        // Create pseudo console
        let size = COORD {
            X: cols as i16,
            Y: rows as i16,
        };

        let hpc = CreatePseudoConsole(size, pty_input_read.get(), pty_output_write.get(), 0)
            .map_err(PtyError::ConPtyCreation)?;
        let hpc = HpcGuard(Some(hpc));

        // Close the handles that the ConPTY now owns (it duplicated them)
        drop(pty_input_read);
        drop(pty_output_write);

        // Prepare process startup
        let mut attr_list_size: usize = 0;
        let _ = InitializeProcThreadAttributeList(
            LPPROC_THREAD_ATTRIBUTE_LIST::default(),
            1,
            0,
            &mut attr_list_size,
        );
        if attr_list_size == 0 {
            // The size-query call is expected to fail with
            // ERROR_INSUFFICIENT_BUFFER while reporting the required size;
            // a zero size means the API itself is misbehaving.
            return Err(PtyError::ProcessSpawn(windows::core::Error::from_win32()));
        }

        let mut attr_list_buffer = vec![0u8; attr_list_size];
        let attr_list = LPPROC_THREAD_ATTRIBUTE_LIST(attr_list_buffer.as_mut_ptr() as *mut _);

        InitializeProcThreadAttributeList(attr_list, 1, 0, &mut attr_list_size)
            .map_err(PtyError::ProcessSpawn)?;
        // Delete the attribute list on all paths (success and error) once
        // initialization has succeeded
        let _attr_list_guard = AttrListGuard(attr_list);

        // Associate ConPTY with the process
        const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x00020016;
        UpdateProcThreadAttribute(
            attr_list,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            Some(hpc.get().0 as *const _),
            std::mem::size_of::<HPCON>(),
            None,
            None,
        )
        .map_err(PtyError::ProcessSpawn)?;

        // Prepare startup info
        let mut startup_info = STARTUPINFOEXW {
            StartupInfo: std::mem::zeroed(),
            lpAttributeList: attr_list,
        };
        startup_info.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;

        let mut process_info = PROCESS_INFORMATION::default();

        let cmd = build_shell_command(command, codepage, cwd_prompt_hook);
        let mut cmd_wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();

        // Create process
        CreateProcessW(
            PCWSTR::null(),
            PWSTR(cmd_wide.as_mut_ptr()),
            None,
            None,
            false,
            EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR::null(),
            &startup_info.StartupInfo,
            &mut process_info,
        )
        .map_err(PtyError::ProcessSpawn)?;

        Ok(ConPty {
            hpc: hpc.release(),
            input_write: pty_input_write.release(),
            output_read: pty_output_read.release(),
            process: process_info,
            cols,
            rows,
        })
    }

    /// Resize the pseudo console
    #[allow(dead_code)]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        let size = COORD {
            X: cols as i16,
            Y: rows as i16,
        };

        unsafe {
            ResizePseudoConsole(self.hpc, size).map_err(PtyError::Resize)?;
        }

        self.cols = cols;
        self.rows = rows;
        Ok(())
    }

    /// Resize the pseudo console (immutable version for use with Arc)
    pub fn resize_pty(&self, cols: u16, rows: u16) -> Result<()> {
        let size = COORD {
            X: cols as i16,
            Y: rows as i16,
        };

        unsafe {
            ResizePseudoConsole(self.hpc, size).map_err(PtyError::Resize)?;
        }

        Ok(())
    }

    /// Write bytes to the PTY (input to shell)
    pub fn write(&self, data: &[u8]) -> Result<usize> {
        let mut written: u32 = 0;

        unsafe {
            WriteFile(self.input_write, Some(data), Some(&mut written), None)
                .map_err(|e| PtyError::Write(io::Error::from_raw_os_error(e.code().0 as i32)))?;
        }

        Ok(written as usize)
    }

    /// Read bytes from the PTY (output from shell) - non-blocking
    pub fn read(&self, buffer: &mut [u8]) -> Result<usize> {
        // First check if there's data available using PeekNamedPipe
        let mut available: u32 = 0;
        
        unsafe {
            // Check how many bytes are available
            if PeekNamedPipe(
                self.output_read,
                None,
                0,
                None,
                Some(&mut available),
                None,
            ).is_err() {
                // Pipe error - likely process exited
                return Err(PtyError::Read(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Pipe closed"
                )));
            }
        }

        // If no data available, return 0 (non-blocking)
        if available == 0 {
            return Ok(0);
        }

        // Read available data
        let to_read = (available as usize).min(buffer.len());
        let mut read: u32 = 0;

        unsafe {
            ReadFile(self.output_read, Some(&mut buffer[..to_read]), Some(&mut read), None)
                .map_err(|e| PtyError::Read(io::Error::from_raw_os_error(e.code().0 as i32)))?;
        }

        Ok(read as usize)
    }

    /// Check if the process is still running
    pub fn is_running(&self) -> bool {
        unsafe {
            let result = WaitForSingleObject(self.process.hProcess, 0);
            result.0 != 0 // WAIT_OBJECT_0 = 0 means signaled (exited)
        }
    }

    /// Get the exit code if the process has exited
    #[allow(dead_code)]
    pub fn exit_code(&self) -> Option<u32> {
        if self.is_running() {
            return None;
        }

        let mut exit_code: u32 = 0;
        unsafe {
            if GetExitCodeProcess(self.process.hProcess, &mut exit_code).is_ok() {
                Some(exit_code)
            } else {
                None
            }
        }
    }

    /// Get current size
    #[allow(dead_code)]
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Cancel pending read operations (to unblock reader thread)
    pub fn cancel_read(&self) {
        unsafe {
            let _ = CancelIoEx(self.output_read, None);
        }
    }

    /// Get the output read handle (for cancellation)
    #[allow(dead_code)]
    pub fn output_handle(&self) -> HANDLE {
        self.output_read
    }
}

impl Drop for ConPty {
    fn drop(&mut self) {
        unsafe {
            // Close the pseudo console first
            ClosePseudoConsole(self.hpc);

            // Close handles
            let _ = CloseHandle(self.input_write);
            let _ = CloseHandle(self.output_read);
            let _ = CloseHandle(self.process.hProcess);
            let _ = CloseHandle(self.process.hThread);
        }
    }
}

fn build_shell_command(command: Option<&str>, codepage: Option<u32>, cwd_prompt_hook: bool) -> String {
    match (command, codepage) {
        (Some(cmd), Some(cp)) => {
            let cmd_lower = cmd.to_lowercase();
            if cmd_lower == "cmd.exe" || cmd_lower == "cmd" {
                let hook = if cwd_prompt_hook {
                    format!(" & {}", cmd_cwd_prompt_hook())
                } else {
                    String::new()
                };
                format!("cmd.exe /k \"chcp {} >nul{}\"", cp, hook)
            } else if cmd_lower.contains("powershell") || cmd_lower.contains("pwsh") {
                format!("{} -NoExit -Command \"{}\"", cmd, powershell_startup_script(cwd_prompt_hook))
            } else if cmd_lower.contains("wsl") {
                cmd.to_string()
            } else {
                format!("cmd.exe /k \"chcp {} >nul & {}\"", cp, cmd)
            }
        }
        (Some(cmd), None) => {
            let cmd_lower = cmd.to_lowercase();
            if cmd_lower == "cmd.exe" || cmd_lower == "cmd" {
                if cwd_prompt_hook {
                    format!("cmd.exe /k \"{}\"", cmd_cwd_prompt_hook())
                } else {
                    cmd.to_string()
                }
            } else if cmd_lower.contains("powershell") || cmd_lower.contains("pwsh") {
                format!("{} -NoExit -Command \"{}\"", cmd, powershell_startup_script(cwd_prompt_hook))
            } else {
                cmd.to_string()
            }
        }
        (None, Some(cp)) => {
            let hook = if cwd_prompt_hook {
                format!(" & {}", cmd_cwd_prompt_hook())
            } else {
                String::new()
            };
            format!("cmd.exe /k \"chcp {} >nul{}\"", cp, hook)
        }
        (None, None) => {
            if cwd_prompt_hook {
                format!("cmd.exe /k \"{}\"", cmd_cwd_prompt_hook())
            } else {
                "cmd.exe".to_string()
            }
        }
    }
}

fn cmd_cwd_prompt_hook() -> &'static str {
    "if defined PROMPT (prompt $E]9;9;$P$E\\%PROMPT%) else (prompt $E]9;9;$P$E\\$P$G)"
}

fn powershell_startup_script(cwd_prompt_hook: bool) -> String {
    let mut script = String::new();
    if cwd_prompt_hook {
        script.push_str(concat!(
            "$__wtmux_original_prompt = if (Test-Path Function:\\prompt) { ${function:prompt} } else { { 'PS ' + (Get-Location) + '> ' } }; ",
            "function global:prompt { ",
            "$location = Get-Location; ",
            "$path = if ($location.Provider.Name -eq 'FileSystem') { $location.ProviderPath } else { $location.Path }; ",
            "[Console]::Out.Write(([string][char]27 + ']9;9;' + $path + [string][char]27 + '\\')); ",
            "& $__wtmux_original_prompt ",
            "}; "
        ));
    }
    script.push_str("[Console]::InputEncoding = [System.Text.Encoding]::UTF8; ");
    script.push_str("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8");
    script
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn test_conpty_creation() {
        let pty = ConPty::new(80, 24, Some("cmd.exe /c echo hello"));
        assert!(pty.is_ok());
    }

    #[test]
    fn powershell_sets_input_and_output_encoding() {
        let cmd = build_shell_command(Some("pwsh.exe"), Some(65001), true);

        assert!(cmd.contains("[Console]::InputEncoding = [System.Text.Encoding]::UTF8"));
        assert!(cmd.contains("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8"));
    }

    #[test]
    fn powershell_wraps_prompt_with_cwd_osc() {
        let cmd = build_shell_command(Some("pwsh.exe"), Some(65001), true);

        assert!(cmd.contains("function global:prompt"));
        assert!(cmd.contains("]9;9;"));
        assert!(cmd.contains("$__wtmux_original_prompt"));
    }

    #[test]
    fn cmd_wraps_prompt_with_cwd_osc() {
        let cmd = build_shell_command(Some("cmd.exe"), Some(65001), true);

        assert!(cmd.contains("prompt $E]9;9;$P$E\\"));
        assert!(cmd.contains("%PROMPT%"));
    }

    #[test]
    fn wsl_command_is_not_wrapped() {
        let cmd = build_shell_command(Some("wsl.exe"), Some(65001), true);

        assert_eq!(cmd, "wsl.exe");
    }

    #[test]
    fn powershell_cwd_prompt_hook_can_be_disabled() {
        let cmd = build_shell_command(Some("pwsh.exe"), Some(65001), false);

        assert!(!cmd.contains("function global:prompt"));
        assert!(!cmd.contains("]9;9;"));
        assert!(cmd.contains("[Console]::InputEncoding = [System.Text.Encoding]::UTF8"));
    }

    #[test]
    fn cmd_cwd_prompt_hook_can_be_disabled() {
        let cmd = build_shell_command(Some("cmd.exe"), Some(65001), false);

        assert_eq!(cmd, "cmd.exe /k \"chcp 65001 >nul\"");
    }
}
