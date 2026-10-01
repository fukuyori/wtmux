# Version Update Checklist

`wtmux` の版番号を更新するときに毎回確認するファイルのメモ。

## 毎回更新するファイル

- `Cargo.toml`
  - crate version
- `Cargo.lock`
  - `[[package]] name = "wtmux"` の version
- `installer/wtmux.iss`
  - `MyAppVersion`
- `installer/wtmux.wxs`
  - WiX `Product` version
- `installer/msix/AppxManifest.xml`
  - MSIX identity version (`x.y.z.0`)
- `CHANGELOG.md`
  - 先頭のリリース見出し
- `README.md`
  - version badge
  - release highlights 見出し
- `README.ja.md`
  - version badge
  - release highlights 見出し

## 内容に応じて確認するファイル

- `docs/tutorial-en.md`
  - 特定バージョンを文中で説明している場合
- `docs/tutorial-ja.md`
  - 特定バージョンを文中で説明している場合
- `docs/design-resize-rendering-refactor.md`
  - フェーズ進捗や「`1.x.y` 時点で」の記述がある場合
- その他 `docs/` 配下
  - 特定バージョン番号を本文で参照している場合

## 確認コマンド

版番号を `1.3.8` から次の版へ上げるときは、まず現在値を検索する。

```powershell
rg -n "1\.3\.8" Cargo.toml Cargo.lock README.md README.ja.md CHANGELOG.md docs installer
```

汎用的に「version が入りそうな場所」を見るときは次も使える。

```powershell
rg -n "version =|MyAppVersion|## 1\.|\[1\." Cargo.toml Cargo.lock README.md README.ja.md CHANGELOG.md docs installer
```

## 運用メモ

- `README` と `CHANGELOG` は原則セットで更新する
- `docs/` のバージョン記述は、機能説明と強く結びついている場合だけ更新する
- `Cargo.lock` は `wtmux` パッケージの version だけを変える

## Linux 版のリリース (GitHub Actions)

- `.github/workflows/release-linux.yml` が `.deb` / `.rpm` (x86_64 / aarch64、Ubuntu 22.04 でビルド) を作る
- `Cargo.toml` の版番号と同じタグ (例 `4.1.0`、`v` なし) を push すると起動する
  - タグと `Cargo.toml` の版が違えば失敗する
  - そのタグのリリースが無ければ **ドラフト** で作成する (本文は `CHANGELOG.md` の該当節)
  - 既にあれば Linux のパッケージを追加 (上書き) するだけ
- Windows / macOS の配布物は従来どおり利用者が `-Sign` 付きで作成して同じリリースに追加し、最後にドラフトを公開する
- タグなしの試し実行: Actions > Release (Linux) > Run workflow (`publish` を false のまま)。ビルドと `.deb` の動作確認だけ行い、成果物は実行結果に残る
