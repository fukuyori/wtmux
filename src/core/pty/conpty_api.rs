//! ConPTY entry-point resolution.
//!
//! wtmux's panes run inside a ConPTY. The inbox implementation reached through
//! kernel32 (`conhost.exe`) measures text width per code point, so combining
//! marks, ZWJ emoji sequences, flags and VS16 emoji occupy more cells there
//! than in any modern host terminal. Windows Terminal and WezTerm avoid this by
//! shipping their own `conpty.dll` + `OpenConsole.exe` (microsoft/terminal, MIT)
//! and calling the ConPTY API from that DLL instead. This module lets wtmux do
//! the same: it resolves `CreatePseudoConsole` / `ResizePseudoConsole` /
//! `ClosePseudoConsole` once, preferring a bundled DLL and falling back to
//! kernel32.
//!
//! Search order:
//! 1. `WTMUX_CONPTY=system` forces the kernel32 implementation.
//! 2. `WTMUX_CONPTY_DIR=<dir>` loads `<dir>\conpty.dll`.
//! 3. `conpty.dll` next to the running executable.
//! 4. kernel32.
//!
//! `conpty.dll` locates `OpenConsole.exe` next to itself, so both files must
//! sit in the same directory. A directory is only used when it holds the
//! pair: measured, a `conpty.dll` without `OpenConsole.exe` is not an error
//! at all, `CreatePseudoConsole` succeeds and the pane silently runs on the
//! inbox conhost, so wtmux would report a bundled ConPTY it is not using.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HMODULE};
use windows::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
};

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HPCON);

/// Which ConPTY implementation is in use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Backend {
    /// `conpty.dll` loaded from this path.
    Bundled(PathBuf),
    /// kernel32 / inbox conhost.
    System,
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Backend::Bundled(p) => write!(f, "bundled ({})", p.display()),
            Backend::System => write!(f, "system (kernel32 / conhost)"),
        }
    }
}

pub struct ConPtyApi {
    backend: Backend,
    bundled: Option<BundledFns>,
    // Keep the module alive for the process lifetime (never freed).
    _module: Option<HMODULE>,
}

struct BundledFns {
    create: CreateFn,
    resize: ResizeFn,
    close: CloseFn,
}

// Function pointers and HMODULE are plain values; the DLL stays loaded for the
// process lifetime, so sharing the resolved API across threads is sound.
unsafe impl Send for ConPtyApi {}
unsafe impl Sync for ConPtyApi {}

static API: OnceLock<ConPtyApi> = OnceLock::new();

/// The resolved ConPTY API (loaded on first use).
pub fn api() -> &'static ConPtyApi {
    API.get_or_init(ConPtyApi::resolve)
}

impl ConPtyApi {
    pub fn backend(&self) -> &Backend {
        &self.backend
    }

    fn resolve() -> Self {
        if std::env::var("WTMUX_CONPTY").map(|v| v.eq_ignore_ascii_case("system")) == Ok(true) {
            return Self::system();
        }
        for dir in candidate_dirs() {
            let dll = match check_pair(&dir) {
                PairCheck::Missing => continue,
                PairCheck::DllWithoutHost(dll) => {
                    eprintln!(
                        "[wtmux] {} has no OpenConsole.exe next to it (conpty.dll would silently fall back to the inbox conhost), trying next",
                        dll.display()
                    );
                    continue;
                }
                PairCheck::Complete(dll) => dll,
            };
            match unsafe { Self::load(&dll) } {
                Ok(api) => return api,
                Err(e) => eprintln!("[wtmux] conpty.dll at {} unusable ({}), trying next", dll.display(), e),
            }
        }
        Self::system()
    }

    fn system() -> Self {
        Self { backend: Backend::System, bundled: None, _module: None }
    }

    unsafe fn load(dll: &Path) -> std::result::Result<Self, windows::core::Error> {
        let wide: Vec<u16> = dll.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // Altered search path: dependencies resolve relative to the DLL's own
        // directory, which is also where conpty.dll expects OpenConsole.exe.
        let module = LoadLibraryExW(PCWSTR(wide.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH)?;
        let sym = |name: &[u8]| -> std::result::Result<usize, windows::core::Error> {
            GetProcAddress(module, windows::core::PCSTR(name.as_ptr()))
                .map(|p| p as usize)
                .ok_or_else(windows::core::Error::from_win32)
        };
        let create = sym(b"CreatePseudoConsole\0")?;
        let resize = sym(b"ResizePseudoConsole\0")?;
        let close = sym(b"ClosePseudoConsole\0")?;
        Ok(Self {
            backend: Backend::Bundled(dll.to_path_buf()),
            bundled: Some(BundledFns {
                create: std::mem::transmute::<usize, CreateFn>(create),
                resize: std::mem::transmute::<usize, ResizeFn>(resize),
                close: std::mem::transmute::<usize, CloseFn>(close),
            }),
            _module: Some(module),
        })
    }

    /// # Safety
    /// Same contract as `CreatePseudoConsole`.
    pub unsafe fn create(
        &self,
        size: COORD,
        input: HANDLE,
        output: HANDLE,
        flags: u32,
    ) -> windows::core::Result<HPCON> {
        match &self.bundled {
            Some(f) => {
                let mut hpc = HPCON::default();
                (f.create)(size, input, output, flags, &mut hpc).ok()?;
                Ok(hpc)
            }
            None => CreatePseudoConsole(size, input, output, flags),
        }
    }

    /// # Safety
    /// Same contract as `ResizePseudoConsole`.
    pub unsafe fn resize(&self, hpc: HPCON, size: COORD) -> windows::core::Result<()> {
        match &self.bundled {
            Some(f) => (f.resize)(hpc, size).ok(),
            None => ResizePseudoConsole(hpc, size),
        }
    }

    /// # Safety
    /// Same contract as `ClosePseudoConsole`.
    pub unsafe fn close(&self, hpc: HPCON) {
        match &self.bundled {
            Some(f) => (f.close)(hpc),
            None => ClosePseudoConsole(hpc),
        }
    }
}

use std::os::windows::ffi::OsStrExt;

/// What `dir` holds of the bundled ConPTY pair.
#[derive(Debug, PartialEq, Eq)]
enum PairCheck {
    /// No `conpty.dll`: nothing bundled here.
    Missing,
    /// `conpty.dll` without `OpenConsole.exe`: unusable as a bundled ConPTY.
    DllWithoutHost(PathBuf),
    /// Both files present (their versions are not compared).
    Complete(PathBuf),
}

fn check_pair(dir: &Path) -> PairCheck {
    let dll = dir.join("conpty.dll");
    if !dll.is_file() {
        return PairCheck::Missing;
    }
    if !dir.join("OpenConsole.exe").is_file() {
        return PairCheck::DllWithoutHost(dll);
    }
    PairCheck::Complete(dll)
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("WTMUX_CONPTY_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            dirs.push(dir.to_path_buf());
        }
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_backend_is_always_available() {
        // Whatever the environment, resolution must yield a usable API.
        let api = api();
        match api.backend() {
            Backend::Bundled(p) => assert!(p.is_file()),
            Backend::System => assert!(api.bundled.is_none()),
        }
    }

    /// A scratch directory holding the named empty files.
    fn dir_with(name: &str, files: &[&str]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wtmux_conpty_pair_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for file in files {
            std::fs::write(dir.join(file), b"").unwrap();
        }
        dir
    }

    #[test]
    fn pair_check_needs_both_files() {
        let empty = dir_with("empty", &[]);
        assert_eq!(check_pair(&empty), PairCheck::Missing);

        let host_only = dir_with("host_only", &["OpenConsole.exe"]);
        assert_eq!(check_pair(&host_only), PairCheck::Missing);

        let dll_only = dir_with("dll_only", &["conpty.dll"]);
        assert_eq!(
            check_pair(&dll_only),
            PairCheck::DllWithoutHost(dll_only.join("conpty.dll"))
        );

        let both = dir_with("both", &["conpty.dll", "OpenConsole.exe"]);
        assert_eq!(check_pair(&both), PairCheck::Complete(both.join("conpty.dll")));

        // A directory named like the files does not count.
        let dirs_not_files = dir_with("dirs", &[]);
        std::fs::create_dir(dirs_not_files.join("conpty.dll")).unwrap();
        assert_eq!(check_pair(&dirs_not_files), PairCheck::Missing);

        for dir in [empty, host_only, dll_only, both, dirs_not_files] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn candidate_dirs_include_exe_dir() {
        let exe_dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
        assert!(candidate_dirs().contains(&exe_dir));
    }
}
