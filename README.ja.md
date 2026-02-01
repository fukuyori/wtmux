# wtmux

Windows用のtmuxライクなターミナルマルチプレクサ（Rust製）

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Windows](https://img.shields.io/badge/platform-Windows-blue.svg)](https://www.microsoft.com/windows)

[English README](README.md)

## 特徴

- **tmux互換キーバインド** - おなじみの `Ctrl+B` プレフィックスコマンド
- **複数タブ（ウィンドウ）** - タブの作成、切り替え、名前変更、管理
- **ペイン分割** - 水平・垂直分割、リサイズ対応
- **ペインズーム** - 任意のペインを全画面表示
- **レイアウトプリセット** - 5種類のレイアウト（横均等、縦均等、メイン横、メイン縦、タイル）
- **コピーモード** - vimライクなスクロールバック操作とテキスト選択
- **検索機能** - スクロールバック内をハイライト検索
- **カラースキーム** - 8種類の組み込みテーマ（default、solarized、monokai、nord、dracula、gruvbox、tokyo-night）
- **設定ファイル** - TOML形式の設定ファイル対応
- **ConPTY対応** - Windows標準の擬似端末
- **複数シェル対応** - cmd.exe、PowerShell、PowerShell 7、WSL
- **エンコーディング対応** - UTF-8とShift-JIS（CP932）
- **マウスパススルー** - TUIアプリにマウスイベントを転送（Shiftでwtmuxの選択を使用）

## スクリーンショット

```
┌─[0: cmd]─────────────────┬─[1: pwsh]────────────────┐
│ C:\Users\user>           │ PS C:\Users\user>        │
│                          │                          │
│                          ├──────────────────────────┤
│                          │ user@wsl:~$              │
│                          │                          │
└──────────────────────────┴──────────────────────────┘
 [0] cmd [1] pwsh* [2] wsl                    tokyo-night
```

## 動作要件

- Windows 10 バージョン1809以降（ConPTYサポートが必要）
- Rust 1.70以降（ソースからビルドする場合）

## インストール

### 方法1: リリース版をダウンロード

[Releases](https://github.com/user/wtmux/releases) ページからダウンロード：

- **インストーラー** (`wtmux-x.x.x-setup.exe`) - 一般ユーザー向け推奨
- **ポータブル版** (`wtmux-x.x.x-portable-x64.zip`) - インストール不要、展開して実行するだけ
- **MSI** (`wtmux-x.x.x-x64.msi`) - 企業展開向け

### 方法2: PowerShellインストールスクリプト

```powershell
# ビルドしてインストール
cargo build --release
.\install.ps1

# アンインストール
.\install.ps1 -Uninstall
```

### 方法3: ソースからビルド

```bash
git clone https://github.com/user/wtmux.git
cd wtmux
cargo build --release

# 任意の場所にコピー
copy target\release\wtmux.exe C:\your\bin\path\
```

### インストーラーのビルド

```powershell
# ポータブル版（ZIP）
.\build-portable.ps1

# Inno Setup使用（エンドユーザー向け推奨）
# ダウンロード: https://jrsoftware.org/isinfo.php
.\build-inno-installer.ps1

# WiX Toolset使用（企業展開向け）
# ダウンロード: https://wixtoolset.org/releases/
.\build-installer.ps1
```

## 使い方

```bash
# デフォルト: マルチペインモード
wtmux

# PowerShell 7 + UTF-8
wtmux -7 -u

# WSL
wtmux -w

# シンプルモード（単一ペイン）
wtmux -1

# ヘルプ表示
wtmux --help
```

### コマンドラインオプション

| オプション | 説明 |
|--------|-------------|
| `-1, --simple` | シンプルモード（単一ペイン） |
| `-c, --cmd` | コマンドプロンプトを使用 |
| `-p, --powershell` | Windows PowerShellを使用 |
| `-7, --pwsh` | PowerShell 7を使用 |
| `-w, --wsl` | WSLを使用 |
| `-s, --shell <CMD>` | カスタムシェルコマンド |
| `--sjis` | Shift-JISエンコーディング（デフォルト: UTF-8） |
| `-v, --version` | バージョン表示 |
| `-h, --help` | ヘルプ表示 |

## キーバインド

すべてのコマンドは `Ctrl+B` をプレフィックスキーとして使用します（tmuxと同じ）。

### ウィンドウ（タブ）

| キー | 動作 |
|-----|--------|
| `Ctrl+B, c` | 新規ウィンドウ作成 |
| `Ctrl+B, &` | 現在のウィンドウを閉じる |
| `Ctrl+B, n` | 次のウィンドウ |
| `Ctrl+B, p` | 前のウィンドウ |
| `Ctrl+B, l` | 最後のウィンドウに切り替え |
| `Ctrl+B, 0-9` | 番号でウィンドウを選択 |
| `Ctrl+B, ,` | ウィンドウ名を変更 |

### ペイン

| キー | 動作 |
|-----|--------|
| `Ctrl+B, "` | 水平分割（上下） |
| `Ctrl+B, %` | 垂直分割（左右） |
| `Ctrl+B, x` | 現在のペインを閉じる |
| `Ctrl+B, o` | 次のペイン |
| `Ctrl+B, ;` | 前のペイン |
| `Ctrl+B, ←↑↓→` | 指定方向のペインにフォーカス移動 |
| `Ctrl+B, Ctrl+←↑↓→` | ペインサイズ変更 |
| `Ctrl+B, z` | ペインズーム切り替え |
| `Ctrl+B, Space` | レイアウトプリセット切り替え |
| `Ctrl+B, q` | ペイン番号表示（その後0-9で選択） |
| `Ctrl+B, {` | 前のペインと入れ替え |
| `Ctrl+B, }` | 次のペインと入れ替え |

### コピーモード

| キー | 動作 |
|-----|--------|
| `Ctrl+B, [` | コピーモードに入る |
| `Ctrl+B, /` | 検索モードに入る |

コピーモード中：

| キー | 動作 |
|-----|--------|
| `h/j/k/l` または矢印キー | カーソル移動 |
| `0` / `$` | 行頭 / 行末 |
| `g` / `G` | バッファ先頭 / 末尾 |
| `Ctrl+U` / `Ctrl+D` | 半ページ上 / 下 |
| `Ctrl+B` / `Ctrl+F` | 1ページ上 / 下 |
| `Space` または `v` | 選択開始/切り替え |
| `Enter` または `y` | 選択範囲をコピーして終了 |
| `/` | 前方検索 |
| `?` | 後方検索 |
| `n` / `N` | 次 / 前のマッチ |
| `q` または `Esc` | コピーモード終了 |

### その他

| キー | 動作 |
|-----|--------|
| `Ctrl+B, t` | テーマ選択 |
| `Ctrl+B, r` | カーソル形状をリセット |
| `Ctrl+B, b` | アプリケーションにCtrl+Bを送信 |
| `Esc` | プレフィックスモードをキャンセル |

### 履歴機能

wtmuxには、シェルの履歴機能とは別に、独自のコマンド履歴機能が搭載されています。入力したコマンドを記録し、複雑なコマンドを何度も入力する必要がなくなります。

| キー | 動作 |
|-----|--------|
| `Ctrl+R` | 履歴検索表示 |
| `Enter` | 選択コマンドを実行（現在の入力を置換） |
| `Shift+Enter` | `&&` で追加（前コマンド成功時に実行） |
| `Ctrl+Enter` | `&` で追加（バックグラウンド/並列実行） |

詳細は: https://qiita.com/spumoni/items/7d43ed7e579d99cfda3e

## 設定

wtmuxは `~/.wtmux/config.toml` から設定を読み込みます。

```toml
# デフォルトシェル（省略可）
# オプション: "cmd", "powershell", "pwsh", "wsl", またはフルパス
# shell = "pwsh.exe"

# エンコーディングのコードページ（省略可）
# codepage = 65001  # UTF-8
# codepage = 932    # Shift-JIS

# プレフィックスキー（デフォルト: "C-b" = Ctrl+B）
# prefix_key = "C-a"  # Ctrl+Aに変更

# カラースキーム
# 利用可能: default, solarized-dark, solarized-light, monokai, nord, dracula, gruvbox-dark, tokyo-night
color_scheme = "tokyo-night"

# タブバー設定
[tab_bar]
visible = true

# ステータスバー設定
[status_bar]
visible = true
show_time = true

# ペイン境界線設定
[pane]
border_style = "single"  # single, double, rounded, none

# カーソル設定
[cursor]
shape = "block"          # block, underline, bar
blink = true

# スクロールバックバッファ
[scrollback]
lines = 10000
```

### 利用可能なカラースキーム

- `default` - デフォルトのターミナル色
- `solarized` - Solarized Dark
- `monokai` - Monokai Pro
- `nord` - Nord
- `dracula` - Dracula
- `gruvbox` - Gruvbox Dark
- `tokyo-night` - Tokyo Night

## シェルからwtmuxを検出する

wtmuxは子プロセスが検出できる環境変数を設定します：

```batch
REM cmd.exe
if defined WTMUX echo wtmux内で実行中
```

```powershell
# PowerShell
if ($env:WTMUX) { "wtmux内で実行中" }
```

```bash
# bash/WSL
[ -n "$WTMUX" ] && echo "wtmux内で実行中"
```

## マウスサポート

wtmuxは包括的なマウスサポートを提供しています。

### テキスト選択とコピー

マウスでテキストを選択してクリップボードにコピーできます：

1. **クリックしてドラッグ**でテキストを選択
2. **マウスボタンを離す** - 選択されたテキストが自動的にクリップボードにコピーされます
3. `Ctrl+V` または**右クリック → Paste**で**ペースト**

通常のターミナルと同じ操作でテキスト選択ができます。

### TUIアプリケーションへのマウスパススルー

マウス入力を使用するTUIアプリケーション（htop、mc、マウス対応のvim、crosstermの`EnableMouseCapture`を使用するアプリなど）を実行すると、マウスイベントは自動的にアプリケーションに転送されます。

**仕組み:**
- wtmuxは子アプリケーションがマウストラッキングを有効にしたことを検出します（DECSET 1000/1002/1003）
- ペイン内のマウスイベントはアプリケーションに転送されます
- 223列/行を超える大きなターミナル用にSGR拡張マウスモード（1006）をサポート
- タブバーとステータスバーのクリックは通常通り動作します

**TUIアプリ内でのテキスト選択:**
- マウス対応TUIアプリ内でwtmuxのテキスト選択を使用するには、**Shift**キーを押しながらクリック/ドラッグします
- マウス対応TUIアプリからテキストをコピーしたい場合に便利です

### マウス操作一覧

| 操作 | 通常のシェル | TUIアプリ（マウス有効時） |
|------|------------|------------------------|
| 左ドラッグ | テキスト選択 | アプリに転送 |
| Shift + 左ドラッグ | テキスト選択 | テキスト選択 |
| タブバーを左クリック | タブ切り替え | タブ切り替え |
| 右クリック | コンテキストメニュー（Paste, Zoom, Split等） | コンテキストメニュー |
| スクロールホイール | バッファをスクロール | アプリに転送 |

## tmuxとの比較

| 機能 | tmux | wtmux |
|---------|------|-------|
| プラットフォーム | Unix/Linux/macOS | Windows |
| バックエンド | PTY | ConPTY |
| ウィンドウ/ペイン | ✓ | ✓ |
| キーバインド | ✓ | ✓（互換） |
| コピーモード | ✓ | ✓ |
| 検索 | ✓ | ✓ |
| レイアウトプリセット | ✓ | ✓ |
| 設定ファイル | ✓ | ✓ |
| カラースキーム | ✓ | ✓ |
| マウスサポート | ✓ | ✓ |
| デタッチ/アタッチ | ✓ | 予定 |
| セッション共有 | ✓ | 予定 |
| スクリプティング | ✓ | 予定 |

## プロジェクト構成

```
wtmux/
├── Cargo.toml
├── README.md
├── README.ja.md
├── LICENSE
├── CHANGELOG.md
├── config.example.toml
├── install.ps1
├── build-portable.ps1
├── build-installer.ps1
├── build-inno-installer.ps1
├── installer/
│   ├── wtmux.iss          # Inno Setupスクリプト
│   ├── wtmux.wxs          # WiXスクリプト
│   └── license.rtf
└── src/
    ├── main.rs            # エントリーポイント
    ├── config.rs          # 設定
    ├── copymode.rs        # コピーモード
    ├── history.rs         # コマンド履歴
    ├── core/
    │   ├── pty.rs         # ConPTYラッパー
    │   ├── session.rs     # セッション管理
    │   └── term/
    │       ├── state.rs   # ターミナル状態
    │       └── parser.rs  # VTパーサー
    ├── ui/
    │   ├── keymapper.rs   # キーマッピング
    │   ├── renderer.rs    # 画面描画
    │   └── wm_renderer.rs # マルチペイン描画
    └── wm/
        ├── manager.rs     # ウィンドウマネージャ
        ├── tab.rs         # タブ管理
        ├── pane.rs        # ペイン管理
        └── layout.rs      # レイアウト計算
```

## 既知の制限

- Windowsのみ（ConPTYはWindows専用）
- デタッチ/アタッチは未サポート（将来のリリースで対応予定）
- セッション共有は未サポート

## コントリビュート

コントリビュートを歓迎します！お気軽にPull Requestをお送りください。

## ライセンス

このプロジェクトはMITライセンスの下でライセンスされています。詳細は [LICENSE](LICENSE) ファイルをご覧ください。

## 謝辞

- [tmux](https://github.com/tmux/tmux) - このプロジェクトのインスピレーション
- Windows ConPTYチーム - 擬似端末API
- [crossterm](https://github.com/crossterm-rs/crossterm) - クロスプラットフォームターミナル操作
- [unicode-width](https://github.com/unicode-rs/unicode-width) - Unicode文字幅計算
