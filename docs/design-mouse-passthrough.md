# Mouse Event Passthrough 設計書

## Issue Summary

TUIアプリケーション（crossterm の `EnableMouseCapture` を使用するアプリなど）をwtmux内で実行した場合、マウスイベントが子アプリケーションに渡されない問題。

## 現状の実装

### マウスイベントフロー（現在）
```
Windows Terminal → crossterm → wtmux main.rs → wtmux が全て消費
                                               ├── タブクリック
                                               ├── ペイン選択
                                               ├── テキスト選択
                                               ├── コンテキストメニュー
                                               └── スクロール
```

### 問題点
1. `TerminalModes` にマウスモードの追跡がない
2. `set_private_mode()` でマウス関連のDECモード（1000, 1002, 1003, 1006）を無視している
3. マウスイベントを子プロセス（PTY）に送信する仕組みがない

## 提案する実装

### アプローチ: 自動検出 + Shiftバイパス

tmuxと同様に、子アプリケーションがマウスキャプチャを有効にしたことを検出し、自動的にマウスイベントを転送する。

- **子アプリがマウスモード有効**: マウスイベントを子アプリに転送
- **子アプリがマウスモード無効**: wtmuxが処理（現在の動作）
- **Shiftキー押下中**: 常にwtmuxが処理（テキスト選択等）

これはtmuxの動作（Shiftで子アプリにパススルー）とは逆だが、以下の理由でこちらを採用：
- 多くのユーザーはwtmuxのテキスト選択機能をShift+クリックで期待する（一般的なターミナルの動作）
- TUIアプリを使うユーザーは、マウス操作がそのまま動くことを期待する

### 1. TerminalModes の拡張

```rust
// src/core/term/state.rs

pub struct TerminalModes {
    // ... 既存フィールド ...
    
    /// Mouse tracking modes
    /// 1000 - X10 mouse reporting (click only)
    pub mouse_tracking: bool,
    /// 1002 - Button event mouse tracking (click + drag)
    pub mouse_button_tracking: bool,
    /// 1003 - Any event mouse tracking (all movements)
    pub mouse_any_event: bool,
    /// 1006 - SGR extended mouse mode (allows coordinates > 223)
    pub mouse_sgr_mode: bool,
    /// 1015 - URXVT mouse mode (decimal format)
    pub mouse_urxvt_mode: bool,
}

impl TerminalModes {
    /// Returns true if any mouse tracking mode is enabled
    pub fn mouse_enabled(&self) -> bool {
        self.mouse_tracking || self.mouse_button_tracking || self.mouse_any_event
    }
}
```

### 2. set_private_mode の更新

```rust
// src/core/term/state.rs - set_private_mode()

pub fn set_private_mode(&mut self, mode: u16, enable: bool) {
    match mode {
        // ... 既存のケース ...
        
        // Mouse tracking modes
        1000 => self.modes.mouse_tracking = enable,
        1002 => self.modes.mouse_button_tracking = enable,
        1003 => self.modes.mouse_any_event = enable,
        1006 => self.modes.mouse_sgr_mode = enable,
        1015 => self.modes.mouse_urxvt_mode = enable,
        
        _ => {} // Ignore unknown modes
    }
}
```

### 3. マウスイベントのエンコード

```rust
// src/ui/keymapper.rs に追加

/// Encode mouse event to terminal escape sequence
pub fn encode_mouse_event(
    event: &MouseEvent,
    sgr_mode: bool,
    urxvt_mode: bool,
) -> Vec<u8> {
    let (button, pressed) = match event.kind {
        MouseEventKind::Down(btn) => (mouse_button_code(btn), true),
        MouseEventKind::Up(btn) => (mouse_button_code(btn), false),
        MouseEventKind::Drag(btn) => (mouse_button_code(btn) + 32, true),
        MouseEventKind::Moved => (35, true), // No button, movement only
        MouseEventKind::ScrollUp => (64, true),
        MouseEventKind::ScrollDown => (65, true),
    };
    
    // Add modifier keys
    let mut cb = button;
    if event.modifiers.contains(KeyModifiers::SHIFT) {
        cb += 4;
    }
    if event.modifiers.contains(KeyModifiers::ALT) {
        cb += 8;
    }
    if event.modifiers.contains(KeyModifiers::CONTROL) {
        cb += 16;
    }
    
    // 1-based coordinates for terminal
    let x = event.column + 1;
    let y = event.row + 1;
    
    if sgr_mode {
        // SGR mode: \x1b[<Cb;Cx;Cy;M or m
        let suffix = if pressed { 'M' } else { 'm' };
        format!("\x1b[<{};{};{}{}", cb, x, y, suffix).into_bytes()
    } else if urxvt_mode {
        // URXVT mode: \x1b[Cb;Cx;CyM
        format!("\x1b[{};{};{}M", cb + 32, x, y).into_bytes()
    } else {
        // X10 mode: \x1b[M CbCxCy (encoded as bytes + 32)
        if x <= 223 && y <= 223 {
            vec![0x1b, b'[', b'M', (cb + 32) as u8, (x + 32) as u8, (y + 32) as u8]
        } else {
            vec![] // Coordinates out of range for X10 mode
        }
    }
}

fn mouse_button_code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}
```

### 4. WindowManager へのメソッド追加

```rust
// src/wm/manager.rs

impl WindowManager {
    /// Check if the focused pane has mouse tracking enabled
    pub fn focused_pane_wants_mouse(&self) -> bool {
        self.tabs.get(&self.active_tab)
            .and_then(|tab| tab.focused_pane())
            .map(|pane| pane.session.state.modes.mouse_enabled())
            .unwrap_or(false)
    }
    
    /// Get mouse encoding mode for focused pane
    pub fn focused_pane_mouse_mode(&self) -> (bool, bool) {
        self.tabs.get(&self.active_tab)
            .and_then(|tab| tab.focused_pane())
            .map(|pane| {
                let modes = &pane.session.state.modes;
                (modes.mouse_sgr_mode, modes.mouse_urxvt_mode)
            })
            .unwrap_or((false, false))
    }
    
    /// Convert screen coordinates to pane-relative coordinates
    pub fn screen_to_pane_coords(&self, x: u16, y: u16) -> Option<(u16, u16)> {
        self.tabs.get(&self.active_tab)
            .and_then(|tab| tab.focused_pane())
            .and_then(|pane| {
                let px = pane.x;
                let py = pane.y;
                let pw = pane.width;
                let ph = pane.height;
                
                // Check if coordinates are within pane
                if x >= px && x < px + pw && y >= py && y < py + ph {
                    Some((x - px, y - py))
                } else {
                    None
                }
            })
    }
}
```

### 5. main.rs のマウスイベントハンドラ更新

```rust
// src/main.rs - マウスイベント処理部分

Event::Mouse(mouse_event) => {
    use crossterm::event::{MouseEventKind, MouseButton, KeyModifiers};
    
    // Shiftが押されている場合は常にwtmuxが処理
    let shift_held = mouse_event.modifiers.contains(KeyModifiers::SHIFT);
    
    // コンテキストメニューとセレクタの処理は変更なし
    if selector.visible { /* ... */ }
    if context_menu.visible { /* ... */ }
    
    // 子アプリがマウスモード有効かつShiftなしの場合、イベントを転送
    if !shift_held && wm.focused_pane_wants_mouse() {
        // タブバーとステータスバー以外の領域でのみ転送
        let in_pane_area = mouse_event.row >= wm.tab_bar_height 
            && mouse_event.row < wm.height - wm.status_bar_height;
        
        if in_pane_area {
            if let Some((pane_x, pane_y)) = wm.screen_to_pane_coords(
                mouse_event.column, 
                mouse_event.row - wm.tab_bar_height
            ) {
                let (sgr, urxvt) = wm.focused_pane_mouse_mode();
                let mut adjusted_event = mouse_event.clone();
                adjusted_event.column = pane_x;
                adjusted_event.row = pane_y;
                
                let bytes = KeyMapper::encode_mouse_event(&adjusted_event, sgr, urxvt);
                if !bytes.is_empty() {
                    let _ = wm.write(&bytes);
                }
            }
            continue;
        }
    }
    
    // 通常のwtmuxマウス処理
    match mouse_event.kind {
        MouseEventKind::Down(MouseButton::Left) => { /* ... */ }
        // ...
    }
}
```

### 6. 設定オプション（オプショナル）

```rust
// src/config.rs

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MouseConfig {
    /// Mouse passthrough mode: "auto", "always", "never"
    /// - auto: Pass through when child app requests mouse (default)
    /// - always: Always pass through (wtmux mouse features disabled)
    /// - never: Never pass through (current behavior)
    pub passthrough: String,
}

impl Default for MouseConfig {
    fn default() -> Self {
        Self {
            passthrough: "auto".to_string(),
        }
    }
}
```

## テスト計画

### 手動テスト
1. **vicalc / htop / mc などのマウス対応TUIアプリ**
   - wtmux内で起動し、マウスクリックが正常に動作することを確認
   
2. **Shiftバイパス**
   - TUIアプリ内でShift+クリックでwtmuxのテキスト選択が動作することを確認
   
3. **タブバー・ステータスバー**
   - TUIアプリ起動中もタブバークリックでタブ切り替えできることを確認
   
4. **コンテキストメニュー**
   - TUIアプリ起動中も右クリックメニューが動作することを確認

5. **スクロール**
   - マウスホイールが子アプリに転送されることを確認

### エッジケース
- マウスモードを動的に切り替えるアプリケーション
- ズーム状態でのマウス座標変換
- 複数ペインでの異なるマウスモード設定

## 実装の優先順位

1. **Phase 1**: 基本実装
   - TerminalModes にマウスモード追加
   - set_private_mode 更新
   - マウスイベントエンコーディング
   - 基本的なパススルー処理

2. **Phase 2**: 改善
   - Shiftバイパス
   - 座標変換の最適化
   - 設定オプション

3. **Phase 3**: ドキュメント
   - README更新
   - CHANGELOG追加

## 互換性への影響

- **後方互換**: マウスモードを使わないアプリケーションでは現在と同じ動作
- **破壊的変更なし**: デフォルトの動作が変わるが、Shiftバイパスで従来の操作も可能
