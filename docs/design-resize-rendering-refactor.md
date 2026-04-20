# Resize / Rendering リファクタリング計画

## 背景

ウィンドウのリサイズ、とくに「幅を狭めたあとに広げる」操作において、日本語/CJK文字の間隔が広がる、描画がちらつく、表示の取りこぼしが起きる、といった不具合が発生している。

現状は以下の責務が密結合している。

- `TerminalState::resize()` がリサイズ方針の決定と再フロー実装の両方を持つ
- `ScreenBuffer` が物理行、論理行、scrollbackの責務を同時に持つ
- `WmRenderer` が描画だけでなく、文字幅差分の補正や host terminal の挙動推測まで背負っている
- selection / copy-mode / command history / shell integration がローカル画面バッファの形状に依存している

この状態では、表示崩れを局所修正すると別の副作用が出やすい。

## 目的

以下を満たす設計へ段階的に移行する。

- 描画はできるだけ host terminal に任せる
- wtmux は状態管理、レイアウト、scrollback、selection に集中する
- リサイズ時の再フロー方針を切り替え可能にする
- バグ修正前に、責務境界を明確にする

## バージョンスコープ

この計画の各フェーズは、`wtmux 1.3` の作業範囲の中で進める。

- Phase 1 から Phase 4 まではすべて `1.3.x` 系で実施する
- `1.3.0` 時点で全フェーズ完了を必須とはしない
- `1.3.0` では Phase 1 を実施する
- 完成時期は `1.3.6` 前後を想定する
- 各フェーズの進行は固定のバージョン対応ではなく、進捗状況に応じて進める
- ただし `1.3` 系の設計方針として、本計画に沿って段階的に整理する
- `1.2.x` 系では局所修正よりも安定性維持を優先し、大きな構造変更は持ち込まない

想定としては、`1.3` を「resize / reflow / rendering の責務分離を進めるシリーズ」と位置付ける。

## 非目的

この計画書の段階では、直ちに日本語描画バグを修正すること自体を目標にしない。

- 日本語/CJKの最終修正
- PUA / Nerd Font / Powerline の完全互換
- tmux互換の全機能追加

まずは「安全に直せる構造」をつくる。

## 現状の問題整理

### 1. `resize` と `reflow` が分離されていない

対象:

- [src/core/session.rs](/d:/home/source/rust/wtmux/src/core/session.rs:338)
- [src/core/term/state.rs](/d:/home/source/rust/wtmux/src/core/term/state.rs:114)
- [src/core/term/state.rs](/d:/home/source/rust/wtmux/src/core/term/state.rs:801)

`Session::resize()` は `state.resize()` を呼び、その中でローカル再フローが実行される。結果として、

- PTY に先に任せるのか
- ローカル再フローを優先するのか
- alternate screen はどう扱うのか

がコード上で明示されていない。

### 2. `Row` に責務が詰め込まれている

対象:

- [src/core/term/state.rs](/d:/home/source/rust/wtmux/src/core/term/state.rs:894)

`Row` は現在、以下を兼ねている。

- 描画対象の物理行
- `wrapped` による論理行連結の単位
- scrollback の保存単位

そのため、リサイズで「論理行を再構築したい」のに、操作対象が「描画済み物理行」になっている。

### 3. renderer が賢すぎる

対象:

- [src/ui/wm_renderer.rs](/d:/home/source/rust/wtmux/src/ui/wm_renderer.rs:977)
- [src/ui/renderer.rs](/d:/home/source/rust/wtmux/src/ui/renderer.rs:430)

`ui::renderer` は比較的単純な行描画である一方、`wm_renderer` は multi-pane 対応の中で独自補正を抱え込みやすい構造になっている。描画側が host terminal の文字幅差分を推測して補正すると、ちらつきや取りこぼしの原因になりやすい。

## 目指す設計

### 設計原則

1. 画面モデルは「何が表示されているか」を表す
2. 再フローは「どう並べ替えるか」だけを担当する
3. renderer は「どう描くか」だけを担当する
4. host terminal が持つ文字幅・折り返しの最終判断は、可能な限り host 側に任せる

### 最終的な責務分離

- `core/term/state.rs`
  - terminal state
  - cursor
  - modes
  - selection
  - shell integration
- `core/term/reflow.rs` または `core/term/resize.rs`
  - resize policy
  - local reflow
  - anchor remapping
- `core/term/model.rs` 相当
  - physical row
  - logical line
  - render row/view
- `ui/wm_renderer.rs`
  - pane/frame/layout rendering
  - 行描画は単純な left-to-right 出力
  - host terminal の推測ロジックは持たない

## 段階的なリファクタリング計画

## Phase 1: `resize` と `reflow` の分離

### 目的

`TerminalState::resize()` を「方針を選ぶ窓口」にして、再フローの実装詳細を外に出す。

### 作業

- `state.rs` から以下を切り出す
  - `ReflowAnchor`
  - `reflow_resize()`
  - `extract_reflow_cells()`
  - `display_offset_before_col()`
  - `append_wrapped_line()`
  - `row_has_content()`
- `ResizePolicy` を導入する
- `ResizeOutcome` を導入する

### 想定インターフェース

```rust
pub enum ResizePolicy {
    HostDriven,
    LocalReflow,
    NoReflow,
}

pub struct ResizeOutcome {
    pub primary_cursor: Option<(u16, u16)>,
    pub prompt_anchor: Option<(u16, u16)>,
}
```

### 完了条件

- `TerminalState::resize()` に再フロー実装の本体が残っていない
- resize 方針をコード上で明示できる

## Phase 2: 画面モデルの分離

### 目的

`Row` の多重責務を減らし、論理行と物理行を区別できるようにする。

### 作業

- `PhysicalRow` と `LogicalLine` の概念を導入する
- 既存 `Row` を immediate に置き換えなくてもよいので、まず `LogicalLineView` を導入する
- scrollback は論理行ベースの view を通じて読むように変更する

### この段階で改善されること

- copy-mode
- selection
- history / prompt extraction
- resize 時の再フロー

が、物理折り返しに過剰に依存しなくなる。

### 完了条件

- 「今見えている描画行」と「論理的な1行」がコード上で区別されている
- scrollback 読み出しが物理行直読みでなくなっている

## Phase 3: renderer の単純化

### 目的

`wm_renderer` を host terminal に任せる設計へ寄せる。

### 作業

- 行内 `MoveTo` を使う特殊補正を廃止する
- 行描画を `ui::renderer` と同じ基本モデルに寄せる
  - 行頭へ移動
  - 属性単位で flush
  - 行末だけ空白埋め
- renderer から reflow 知識を追い出す

### 完了条件

- `wm_renderer` が文字幅の最終補正をしない
- renderer が `RenderRow` 相当のビューだけを受け取る構造になっている

## Phase 4: resize 方針の明示

### 目的

最終的に resize の振る舞いを設計として固定する。

### 候補

#### A. HostDriven

- 先に PTY / host terminal を resize
- その後の出力で画面を更新
- ローカル再フローは scrollback 補助用途に限定

長所:

- host terminal の文字幅判断をそのまま使える
- 日本語/CJK で破綻しにくい

短所:

- ローカルバッファの見え方とホスト再描画の同期設計が必要

#### B. LocalReflow

- 現在のように wtmux 側でバッファを再配置する

長所:

- ローカルモデルだけで説明できる

短所:

- CJK、emoji、PUA で host terminal と不一致が起きやすい

### 推奨

最終的には `HostDriven` を主設計に寄せる。

## 実施順序

最小リスクの順番は以下。

1. Phase 1 のみ実施
2. Phase 1 完了後、`HostDriven` と `LocalReflow` を切り替えられるようにする
3. そのあと Phase 2
4. 続いて Phase 3
5. 最後に Phase 4 の設計判断を固定する

## マイルストーン

### Milestone A

- `resize` と `reflow` のコード分離完了
- 振る舞いは現状維持
- `1.3` 系の最初の土台
- 実施バージョン: `1.3.0`

### Milestone B

- 論理行ビュー導入
- selection / copy-mode が論理行ベースで読める
- `1.3` 系で内部モデルの分離を開始

### Milestone C

- `wm_renderer` の特殊補正撤去
- 行描画の責務単純化
- `1.3` 系で renderer を host terminal 寄りへ整理

### Milestone D

- resize 方針を `HostDriven` ベースへ統一
- `1.3` 系の最終到達点

## リスク

### 1. テスト不足

現状のユニットテストは resize の一部ケースを押さえているが、実際の host terminal と一致するかまでは保証していない。

必要な補強:

- mixed ASCII + CJK
- scrollback を含む resize
- prompt anchor を含む resize
- alternate screen
- pane split / zoom 付き resize

### 2. 段階的移行中の二重モデル化

`Row` と `LogicalLineView` が共存する期間は複雑度が上がる。これは一時的コストとして許容する。

### 3. 方針未確定のまま局所最適化が混入すること

Phase 1 完了前に描画や再フローへ局所パッチを入れると、責務分離の効果が薄れる。以後の修正は原則として方針分離後に行う。

## 成功条件

- resize 時の不具合を renderer 側か reflow 側かに即座に切り分けられる
- host terminal 依存の問題と、ローカルモデルの問題を混同しない
- 日本語/CJK 問題に対し、局所パッチではなく方針に沿った修正ができる

## 結論

いきなり日本語描画バグを直すのではなく、先に以下を行う。

1. `resize` と `reflow` を分離する
2. 論理行と物理行を分ける
3. renderer を単純化する
4. resize 方針を `HostDriven` ベースで再設計する

最初の着手点は `Phase 1` とする。
