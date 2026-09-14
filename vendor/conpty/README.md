# vendor/conpty — bundled ConPTY (conpty.dll + OpenConsole.exe)

Place the following files in this directory to bundle a modern ConPTY with
wtmux. The packaging scripts (`build-portable.ps1`, `build-inno-installer.ps1`,
`build-installer.ps1`, `build-msix.ps1`) pick them up automatically and install
them next to `wtmux.exe`; when they are absent the packages are built without
them and wtmux falls back to the inbox `conhost.exe`.

| File | Purpose |
|---|---|
| `conpty.dll` | ConPTY API (`CreatePseudoConsole` etc.) loaded by wtmux |
| `OpenConsole.exe` | The console host started by `conpty.dll`; must sit in the same directory |
| `LICENSE-ConPTY.txt` | MIT license notice for the two files above (kept in git) |

The binaries are ignored by git (`*.dll`, `*.exe` in `.gitignore`).

## Where to get them

Both files are build artifacts of [microsoft/terminal](https://github.com/microsoft/terminal)
(MIT license), published by Microsoft as the NuGet package
`Microsoft.Windows.Console.ConPTY` (owner `Microsoft.Terminal`). The same
`.nupkg` is attached to every Windows Terminal GitHub release, e.g.
`Microsoft.Windows.Console.ConPTY.1.24.260710001.nupkg` on
[v1.24.11911.0](https://github.com/microsoft/terminal/releases/tag/v1.24.11911.0).
A `.nupkg` is a ZIP; the x64 files are at

```
runtimes/win-x64/native/conpty.dll
build/native/runtimes/x64/OpenConsole.exe
```

(`win-x86` / `win-arm64` and `x86` / `arm64` for the other architectures).
Both are Authenticode-signed by Microsoft; check with
`Get-AuthenticodeSignature` after extracting. Windows Terminal and WezTerm
ship the same pair, so copying it from either install also works.

Record the version you bundled here so it can be re-measured after updates
(`docs/design-width-model.md`):

- 2026-09-14: 1.24.2607.10001 from `Microsoft.Windows.Console.ConPTY.1.24.260710001.nupkg`
  (Windows Terminal v1.24.11911.0 release asset); all ConPTY harness tests pass with it.

## Why

The inbox `conhost.exe` counts text width per code point, so combining marks,
VS16 emoji, ZWJ sequences and flags occupy extra cells inside the pane's
ConPTY and positioned output after them lands a column off. OpenConsole
passes the application's sequences through and forwards DSR to wtmux. See
`docs/design-width-model.md`.
