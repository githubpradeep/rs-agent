//! Mouse hit-rects from the last compute_view / render pass (herdr ViewState).

use ratatui::layout::Rect;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    Thinking { msg_idx: usize },
    Tool { msg_idx: usize, tool_idx: usize },
    Toast,
    Help,
    FleetRow { index: usize },
    SessionRow { index: usize },
    OutputTab { index: usize },
    ModalDismiss,
    PaletteItem { index: usize },
}

#[derive(Debug, Clone)]
pub struct HitRect {
    pub rect: Rect,
    pub target: HitTarget,
}

#[derive(Debug, Default, Clone)]
pub struct HitMap {
    pub hits: Vec<HitRect>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.hits.clear();
    }

    pub fn push(&mut self, rect: Rect, target: HitTarget) {
        if rect.width > 0 && rect.height > 0 {
            self.hits.push(HitRect { rect, target });
        }
    }

    pub fn hit_at(&self, col: u16, row: u16) -> Option<&HitTarget> {
        // Last wins (overlays registered after content).
        for h in self.hits.iter().rev() {
            if col >= h.rect.x
                && col < h.rect.x.saturating_add(h.rect.width)
                && row >= h.rect.y
                && row < h.rect.y.saturating_add(h.rect.height)
            {
                return Some(&h.target);
            }
        }
        None
    }

    /// Register a chat content line as a 1-row hit at absolute screen coords.
    pub fn push_line(&mut self, x: u16, y: u16, width: u16, target: HitTarget) {
        self.push(
            Rect {
                x,
                y,
                width,
                height: 1,
            },
            target,
        );
    }
}
