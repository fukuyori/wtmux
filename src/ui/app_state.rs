//! Exclusive modal UI state for the window-manager event loop.

use std::time::Instant;

use crate::copymode::CopyMode;
use crate::history::HistorySelector;
use crate::wm::WindowManager;

use super::{AgentDashboard, ContextMenu, WindowSelector, WmOverlay, WmRenderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UiMode {
    Normal,
    HistorySelector,
    ThemeSelector,
    WindowSelector,
    AgentDashboard,
    PaneNumbers,
    CopyMode,
    Rename,
    ContextMenu,
}

pub(crate) struct WmAppState {
    pub(crate) mode: UiMode,
    pub(crate) history_selector: Option<HistorySelector>,
    pub(crate) theme_selector_index: usize,
    pub(crate) window_selector: WindowSelector,
    pub(crate) agent_dashboard: AgentDashboard,
    pub(crate) pane_numbers_started: Instant,
    pub(crate) copy_mode: CopyMode,
    pub(crate) rename_buffer: String,
    pub(crate) context_menu: ContextMenu,
}

impl WmAppState {
    pub(crate) fn new() -> Self {
        Self {
            mode: UiMode::Normal,
            history_selector: None,
            theme_selector_index: 0,
            window_selector: WindowSelector::new(),
            agent_dashboard: AgentDashboard::new(),
            pane_numbers_started: Instant::now(),
            copy_mode: CopyMode::new(),
            rename_buffer: String::new(),
            context_menu: ContextMenu::new(),
        }
    }

    pub(crate) fn history_selector_mut(&mut self) -> &mut HistorySelector {
        self.history_selector
            .get_or_insert_with(HistorySelector::new)
    }

    pub(crate) fn close_mode(&mut self) {
        if let Some(selector) = self.history_selector.as_mut() {
            selector.hide();
        }
        self.window_selector.close();
        self.agent_dashboard.close();
        self.copy_mode.exit();
        self.rename_buffer.clear();
        self.context_menu.hide();
        self.mode = UiMode::Normal;
    }

    pub(crate) fn render(
        &self,
        renderer: &mut WmRenderer,
        wm: &WindowManager,
        themes: &[&'static str],
    ) -> std::io::Result<()> {
        let overlay = match self.mode {
            UiMode::Normal => None,
            UiMode::HistorySelector => self
                .history_selector
                .as_ref()
                .filter(|selector| selector.visible)
                .map(WmOverlay::History),
            UiMode::ThemeSelector => Some(WmOverlay::ThemeSelector {
                themes,
                selected: self.theme_selector_index,
            }),
            UiMode::WindowSelector => Some(WmOverlay::WindowSelector(&self.window_selector)),
            UiMode::AgentDashboard => Some(WmOverlay::AgentDashboard(&self.agent_dashboard)),
            UiMode::PaneNumbers => Some(WmOverlay::PaneNumbers),
            UiMode::CopyMode => Some(WmOverlay::CopyMode(&self.copy_mode)),
            UiMode::Rename => Some(WmOverlay::Rename(&self.rename_buffer)),
            UiMode::ContextMenu => Some(WmOverlay::ContextMenu(&self.context_menu)),
        };
        renderer.render_scene(wm, overlay)
    }

    pub(crate) fn defers_background_render(&self) -> bool {
        matches!(
            self.mode,
            UiMode::CopyMode | UiMode::Rename | UiMode::ContextMenu
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_mode_clears_all_modal_state() {
        let mut ui = WmAppState::new();
        ui.mode = UiMode::ContextMenu;
        ui.history_selector_mut().show();
        ui.window_selector.visible = true;
        ui.copy_mode.active = true;
        ui.rename_buffer = "name".to_string();
        ui.context_menu.visible = true;

        ui.close_mode();

        assert_eq!(ui.mode, UiMode::Normal);
        assert!(!ui.history_selector.as_ref().unwrap().visible);
        assert!(!ui.window_selector.visible);
        assert!(!ui.copy_mode.active);
        assert!(ui.rename_buffer.is_empty());
        assert!(!ui.context_menu.visible);
    }
}
