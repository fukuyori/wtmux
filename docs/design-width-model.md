# 文字幅モデルの設計

wtmux のグリッドが 1 文字に割り当てるセル数は、隣接する端末層と一致していなければならない。
一致しない文字の後ろで絶対位置指定（CUP / CHA）が来ると、その分だけ表示がずれる。
この文書は 2026-09-14 の実測に基づいて、どの層が何を判定し、wtmux がどう合わせるかを定める。

## 経路と判定者

```
子アプリ → 内側 ConPTY → wtmux グリッド → 外側 ConPTY → ホスト端末（描画）
```

| 層 | 実体 | 幅の判定 |
|---|---|---|
| 内側 ConPTY（標準） | kernel32 `CreatePseudoConsole` → 受信箱 `conhost.exe`（26100） | コードポイント単位。結合文字・VS16・ZWJ・国旗が余分なセルを取る。絶対位置を自分の勘定で詰め直す |
| 内側 ConPTY（同梱） | `conpty.dll` + `OpenConsole.exe`（microsoft/terminal 1.22 以降） | 書記素単位。アプリの位置指定をそのまま通し、DSR をホスト（wtmux）へ転送する |
| 外側 ConPTY | ホスト端末が同梱する OpenConsole | 同上。DSR はホスト端末へ転送 |
| ホスト端末 | Windows Terminal 1.24 / WezTerm 1.22 | 書記素単位。VS16 の扱いだけ両者で異なる |

測定手段は `tools/width_probe.py`（`a` + 対象 + DSR で進んだ列を読む）と、
`src/core/session.rs` の ignored テスト `conpty_ambiguous_width_probe` /
`conpty_width_divergence_dump`。

## 実測結果（要点）

| 列 | 受信箱 conhost | OpenConsole / WezTerm | Windows Terminal | unicode-width 0.1 |
|---|---|---|---|---|
| East Asian Ambiguous（※①○ など 1285 文字） | 1 | 1 | 1 | 1 |
| a + U+0300 | 2 | 1 | 1 | 1 |
| ❤ + U+FE0F | 2 | 1 | 2 | 1 |
| 👨‍👩‍👧（ZWJ 列） | 8 | 2 | 2 | 6 |
| 🇯🇵 | 4 | 2 | 2 | 2 |
| ｶﾞ（U+FF76 U+FF9E） | 2 | 2 | 2 | 1 |
| か + U+3099 | 4 | 2 | 2 | 2 |

曖昧幅文字はどの層でも半角で一致しており、調整の対象ではない。

## 方針

1. **内側は同梱 ConPTY を使う。** `src/core/pty/conpty_api.rs` が `conpty.dll` を
   `WTMUX_CONPTY_DIR` → exe と同じディレクトリの順に探し、無ければ kernel32 に戻る。
   OpenConsole はアプリの出力をほぼ素通しし、DSR を wtmux に転送するので、
   子アプリは wtmux のグリッドと同じ答えを見る。受信箱 conhost の幅モデルを
   wtmux 側で再現する必要はなくなる。
2. **wtmux の幅関数はホスト端末の書記素モデルに合わせる。** `unicode-width` を基礎に、
   実測した全端末が一致して異なる答えを返す点だけを `width.rs` で上書きする。
   現在は半角濁点・半濁点と半角ハングル充填文字（U+FF9E〜FFA0）を 1 セルとする。
   ZWJ の直後の文字は `put_char` が直前の書記素に結合し、ZWJ 列を 2 セルに保つ。
   **端末が 1 つの書記素クラスタとして描くものは 1 セルに入れる。** 地域指示子 2 つ
   （🇯🇵）と半角カナ + 半角（半）濁点（ｶﾞ）は `extend_cluster_before_cursor` で
   幅 2 の 1 セルにまとめる。レンダラは多バイト文字のセルごとにカーソルを置き直す
   （`row_stream::is_risky_cell`）ため、クラスタを 2 セルに分けると後半が別の位置指定付き
   書き込みになり、WezTerm ではその直後の文字が消える（`tools/host_cluster_probe.py` を
   ホスト端末で直接実行すると再現できる）。
3. **端末ごとに違う点は起動時に測る。** VS16 付き絵文字（❤️）は Windows Terminal が
   2 セル、WezTerm が 1 セル。`ui::host_probe` が代替画面に入った直後に
   `❤\u{FE0F}` を書いて DSR で列を読み、`width::set_vs16_emoji_wide` に反映する。
   応答が無ければ 1 セル（unicode-width の答え）のままにする。対象文字は
   UCD `emoji-data.txt` から生成した `emoji_text_default.rs`
   （Emoji=Yes かつ Emoji_Presentation=No）に限る。

## リサイズ時の画面変形（ConsoleBuffer ポリシー）

ConPTY のバッファは、ペインのリサイズ時に次のように動く（`D:\tmp\conpty_resize_probe.py` で
受信箱 conhost 26100 と OpenConsole 1.24 を実測。両者は同一）。

| 操作 | バッファの動き |
|---|---|
| 幅の変更 | 表示中の行を折り返し直し、カーソルは論理行上の位置に追従 |
| 高さを縮める | カーソルが見えなくなる分だけ上の行を捨てる。収まっていれば行は動かない |
| 高さを広げる | 下に空行を足すだけ。スクロールバックから引き戻さない |
| 送ってくるもの | 受信箱 conhost は画面全体を絶対位置で描き直す。OpenConsole は**何も送らない** |

Console API を使うアプリ（PSReadLine、cmd）はこのバッファの座標でカーソルを置くので、
wtmux の画面も同じ規則で変形しなければならない。従来の `HostDriven` は末尾の行を表示し、
広げたときにスクロールバックから引き戻していたが、受信箱 conhost の描き直しに上書きされて
問題が見えなかった。OpenConsole では描き直しが無いので、Windows の既定を
`ResizePolicy::ConsoleBuffer`（`resize.rs` の `console_buffer_resize_screen`）に変えた。
副作用として、ペインを広げてもプロンプトは最下行に降りてこない（Windows Terminal と同じ）。

## 触らないもの

- 曖昧幅文字。locale-eaw の EAW-CONSOLE をホスト側に入れると、逆に ConPTY との不一致を生む。
- ハングル字母 U+1160〜11FF（WezTerm 0、Windows Terminal 1）。ホスト間で割れており、
  実用上の出現頻度も低いので unicode-width のまま。
- U+17A4 / U+17D8（unicode-width 0.1 が 2 / 3 と返す）。クレート更新で解消を待つ。

## 再測定

Windows、Windows Terminal、WezTerm、OpenConsole の更新後は
`cargo test conpty_ambiguous_width_probe -- --ignored --nocapture`
（環境変数は同テストのコメント参照）と、ホスト端末内での
`python tools/width_probe.py <EastAsianWidth.txt> <out> 65001 - seq`
を再実行して、この文書の表を更新する。
