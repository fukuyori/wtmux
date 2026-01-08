//! Layout - Manages pane arrangement within a tab

use super::pane::PaneId;

/// Direction of split
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SplitDirection {
    Horizontal, // Split left/right (vertical line)
    Vertical,   // Split top/bottom (horizontal line)
}

/// Layout preset types
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum LayoutType {
    /// Current custom layout
    Custom,
    /// Even horizontal (all panes side by side)
    EvenHorizontal,
    /// Even vertical (all panes stacked)
    EvenVertical,
    /// Main pane on top, rest horizontal below
    MainHorizontal,
    /// Main pane on left, rest vertical on right
    MainVertical,
    /// Tiled (grid)
    Tiled,
}

impl LayoutType {
    /// Get next layout type
    pub fn next(self) -> Self {
        match self {
            LayoutType::Custom => LayoutType::EvenHorizontal,
            LayoutType::EvenHorizontal => LayoutType::EvenVertical,
            LayoutType::EvenVertical => LayoutType::MainHorizontal,
            LayoutType::MainHorizontal => LayoutType::MainVertical,
            LayoutType::MainVertical => LayoutType::Tiled,
            LayoutType::Tiled => LayoutType::EvenHorizontal,
        }
    }
}

/// Layout node - binary tree structure
#[derive(Clone)]
pub enum Layout {
    /// A leaf node containing a pane
    Pane(PaneId),
    /// A split containing two child layouts
    Split {
        direction: SplitDirection,
        /// Ratio of first child (0.0 - 1.0)
        ratio: f32,
        first: Box<Layout>,
        second: Box<Layout>,
    },
}

impl Layout {
    /// Create a new layout with a single pane
    pub fn new(pane_id: PaneId) -> Self {
        Layout::Pane(pane_id)
    }

    /// Get current layout type (heuristic)
    #[allow(dead_code)]
    pub fn get_layout_type(&self) -> LayoutType {
        // Simple heuristic - just return Custom for now
        // The actual type is determined by the preset used
        LayoutType::Custom
    }

    /// Create layout from preset
    pub fn from_preset(layout_type: LayoutType, pane_ids: &[PaneId]) -> Self {
        if pane_ids.is_empty() {
            return Layout::Pane(1);
        }
        if pane_ids.len() == 1 {
            return Layout::Pane(pane_ids[0]);
        }

        match layout_type {
            LayoutType::Custom | LayoutType::EvenHorizontal => {
                Self::even_horizontal(pane_ids)
            }
            LayoutType::EvenVertical => {
                Self::even_vertical(pane_ids)
            }
            LayoutType::MainHorizontal => {
                Self::main_horizontal(pane_ids)
            }
            LayoutType::MainVertical => {
                Self::main_vertical(pane_ids)
            }
            LayoutType::Tiled => {
                Self::tiled(pane_ids)
            }
        }
    }

    /// Even horizontal layout (all panes side by side)
    fn even_horizontal(pane_ids: &[PaneId]) -> Self {
        Self::build_even(pane_ids, SplitDirection::Horizontal)
    }

    /// Even vertical layout (all panes stacked)
    fn even_vertical(pane_ids: &[PaneId]) -> Self {
        Self::build_even(pane_ids, SplitDirection::Vertical)
    }

    /// Build even layout recursively
    fn build_even(pane_ids: &[PaneId], direction: SplitDirection) -> Self {
        if pane_ids.len() == 1 {
            return Layout::Pane(pane_ids[0]);
        }
        if pane_ids.len() == 2 {
            return Layout::Split {
                direction,
                ratio: 0.5,
                first: Box::new(Layout::Pane(pane_ids[0])),
                second: Box::new(Layout::Pane(pane_ids[1])),
            };
        }

        // Split in half
        let mid = pane_ids.len() / 2;
        let ratio = mid as f32 / pane_ids.len() as f32;
        
        Layout::Split {
            direction,
            ratio,
            first: Box::new(Self::build_even(&pane_ids[..mid], direction)),
            second: Box::new(Self::build_even(&pane_ids[mid..], direction)),
        }
    }

    /// Main horizontal (main on top, rest below in a row)
    fn main_horizontal(pane_ids: &[PaneId]) -> Self {
        if pane_ids.len() <= 2 {
            return Self::even_vertical(pane_ids);
        }

        Layout::Split {
            direction: SplitDirection::Vertical,
            ratio: 0.6,
            first: Box::new(Layout::Pane(pane_ids[0])),
            second: Box::new(Self::even_horizontal(&pane_ids[1..])),
        }
    }

    /// Main vertical (main on left, rest on right stacked)
    fn main_vertical(pane_ids: &[PaneId]) -> Self {
        if pane_ids.len() <= 2 {
            return Self::even_horizontal(pane_ids);
        }

        Layout::Split {
            direction: SplitDirection::Horizontal,
            ratio: 0.6,
            first: Box::new(Layout::Pane(pane_ids[0])),
            second: Box::new(Self::even_vertical(&pane_ids[1..])),
        }
    }

    /// Tiled layout (grid)
    fn tiled(pane_ids: &[PaneId]) -> Self {
        if pane_ids.len() <= 2 {
            return Self::even_horizontal(pane_ids);
        }

        // Calculate grid dimensions
        let count = pane_ids.len();
        let cols = (count as f64).sqrt().ceil() as usize;
        let rows = (count + cols - 1) / cols;

        // Build rows
        let mut row_layouts = Vec::new();
        for row in 0..rows {
            let start = row * cols;
            let end = (start + cols).min(count);
            if start < count {
                row_layouts.push(Self::even_horizontal(&pane_ids[start..end]));
            }
        }

        // Stack rows vertically
        Self::stack_layouts(&row_layouts, SplitDirection::Vertical)
    }

    /// Stack layouts with even ratios
    fn stack_layouts(layouts: &[Layout], direction: SplitDirection) -> Self {
        if layouts.is_empty() {
            return Layout::Pane(1);
        }
        if layouts.len() == 1 {
            return layouts[0].clone();
        }
        if layouts.len() == 2 {
            return Layout::Split {
                direction,
                ratio: 0.5,
                first: Box::new(layouts[0].clone()),
                second: Box::new(layouts[1].clone()),
            };
        }

        let mid = layouts.len() / 2;
        let ratio = mid as f32 / layouts.len() as f32;

        Layout::Split {
            direction,
            ratio,
            first: Box::new(Self::stack_layouts(&layouts[..mid], direction)),
            second: Box::new(Self::stack_layouts(&layouts[mid..], direction)),
        }
    }

    /// Split a pane in this layout
    pub fn split(&mut self, target_pane: PaneId, new_pane: PaneId, direction: SplitDirection) -> bool {
        match self {
            Layout::Pane(id) => {
                if *id == target_pane {
                    *self = Layout::Split {
                        direction,
                        ratio: 0.5,
                        first: Box::new(Layout::Pane(target_pane)),
                        second: Box::new(Layout::Pane(new_pane)),
                    };
                    true
                } else {
                    false
                }
            }
            Layout::Split { first, second, .. } => {
                first.split(target_pane, new_pane, direction) ||
                second.split(target_pane, new_pane, direction)
            }
        }
    }

    /// Remove a pane from the layout, returns the remaining layout or None if empty
    pub fn remove(&mut self, pane_id: PaneId) -> Option<Layout> {
        match self {
            Layout::Pane(id) => {
                if *id == pane_id {
                    None
                } else {
                    Some(self.clone())
                }
            }
            Layout::Split { first, second, .. } => {
                // Try to remove from first
                let first_result = first.remove(pane_id);
                let second_result = second.remove(pane_id);
                
                match (first_result, second_result) {
                    (None, Some(remaining)) => Some(remaining),
                    (Some(remaining), None) => Some(remaining),
                    (Some(f), Some(s)) => {
                        *first = Box::new(f);
                        *second = Box::new(s);
                        Some(self.clone())
                    }
                    (None, None) => None,
                }
            }
        }
    }

    /// Get all pane IDs in this layout
    pub fn pane_ids(&self) -> Vec<PaneId> {
        match self {
            Layout::Pane(id) => vec![*id],
            Layout::Split { first, second, .. } => {
                let mut ids = first.pane_ids();
                ids.extend(second.pane_ids());
                ids
            }
        }
    }

    /// Calculate positions and sizes for all panes
    pub fn calculate_positions(&self, x: u16, y: u16, width: u16, height: u16) -> Vec<(PaneId, u16, u16, u16, u16)> {
        match self {
            Layout::Pane(id) => vec![(*id, x, y, width, height)],
            Layout::Split { direction, ratio, first, second } => {
                let mut positions = Vec::new();
                
                match direction {
                    SplitDirection::Horizontal => {
                        // Split left/right
                        let first_width = ((width as f32) * ratio) as u16;
                        let second_width = width - first_width;
                        
                        positions.extend(first.calculate_positions(x, y, first_width, height));
                        positions.extend(second.calculate_positions(x + first_width, y, second_width, height));
                    }
                    SplitDirection::Vertical => {
                        // Split top/bottom
                        let first_height = ((height as f32) * ratio) as u16;
                        let second_height = height - first_height;
                        
                        positions.extend(first.calculate_positions(x, y, width, first_height));
                        positions.extend(second.calculate_positions(x, y + first_height, width, second_height));
                    }
                }
                
                positions
            }
        }
    }

    /// Find pane in a direction from current pane
    pub fn find_neighbor(&self, from: PaneId, direction: SplitDirection, forward: bool) -> Option<PaneId> {
        let positions = self.calculate_positions(0, 0, 100, 100);
        let current = positions.iter().find(|(id, _, _, _, _)| *id == from)?;
        let (_, cur_x, cur_y, cur_w, cur_h) = *current;
        
        // Helper to check if two ranges overlap
        let ranges_overlap = |a_start: u16, a_len: u16, b_start: u16, b_len: u16| -> bool {
            let a_end = a_start + a_len;
            let b_end = b_start + b_len;
            a_start < b_end && b_start < a_end
        };
        
        match direction {
            SplitDirection::Horizontal => {
                // Look left or right - must have overlapping Y range
                if forward {
                    // Look right
                    positions.iter()
                        .filter(|(id, x, y, _, h)| {
                            *id != from && 
                            *x > cur_x &&
                            ranges_overlap(cur_y, cur_h, *y, *h)
                        })
                        .min_by_key(|(_, x, _, _, _)| *x)
                        .map(|(id, _, _, _, _)| *id)
                } else {
                    // Look left
                    positions.iter()
                        .filter(|(id, x, y, _, h)| {
                            *id != from && 
                            *x < cur_x &&
                            ranges_overlap(cur_y, cur_h, *y, *h)
                        })
                        .max_by_key(|(_, x, _, _, _)| *x)
                        .map(|(id, _, _, _, _)| *id)
                }
            }
            SplitDirection::Vertical => {
                // Look up or down - must have overlapping X range
                if forward {
                    // Look down
                    positions.iter()
                        .filter(|(id, x, y, w, _)| {
                            *id != from && 
                            *y > cur_y &&
                            ranges_overlap(cur_x, cur_w, *x, *w)
                        })
                        .min_by_key(|(_, _, y, _, _)| *y)
                        .map(|(id, _, _, _, _)| *id)
                } else {
                    // Look up
                    positions.iter()
                        .filter(|(id, x, y, w, _)| {
                            *id != from && 
                            *y < cur_y &&
                            ranges_overlap(cur_x, cur_w, *x, *w)
                        })
                        .max_by_key(|(_, _, y, _, _)| *y)
                        .map(|(id, _, _, _, _)| *id)
                }
            }
        }
    }

    /// Adjust the split ratio
    pub fn adjust_ratio(&mut self, pane_id: PaneId, delta: f32) -> bool {
        match self {
            Layout::Pane(_) => false,
            Layout::Split { first, second, ratio, .. } => {
                let first_ids = first.pane_ids();
                let second_ids = second.pane_ids();
                
                if first_ids.contains(&pane_id) || second_ids.contains(&pane_id) {
                    *ratio = (*ratio + delta).clamp(0.1, 0.9);
                    true
                } else {
                    first.adjust_ratio(pane_id, delta) || second.adjust_ratio(pane_id, delta)
                }
            }
        }
    }

    /// Resize in a specific direction (tmux compatible)
    /// 
    /// tmux behavior for a pane:
    /// - Up arrow: First try to move the BOTTOM boundary up, if no bottom boundary, try TOP boundary up
    /// - Down arrow: First try to move the BOTTOM boundary down, if no bottom boundary, try TOP boundary down
    /// - Left arrow: First try to move the RIGHT boundary left, if no right boundary, try LEFT boundary left
    /// - Right arrow: First try to move the RIGHT boundary right, if no right boundary, try LEFT boundary right
    /// 
    /// arrow_up_or_left: true = up/left arrow, false = down/right arrow
    pub fn resize_in_direction(&mut self, pane_id: PaneId, target_dir: SplitDirection, arrow_up_or_left: bool) -> bool {
        // First try the "second side" boundary (bottom/right) - pane must be directly in "first"
        // Then try the "first side" boundary (top/left) - pane must be directly in "second"
        
        // Try bottom/right boundary first (where pane is in first half)
        if self.try_move_adjacent_boundary(pane_id, target_dir, true, arrow_up_or_left) {
            return true;
        }
        
        // Try top/left boundary (where pane is in second half)
        if self.try_move_adjacent_boundary(pane_id, target_dir, false, arrow_up_or_left) {
            return true;
        }
        
        false
    }

    /// Try to move a boundary that is directly adjacent to the pane
    /// look_for_second_boundary: true = looking for bottom/right boundary (pane directly in first)
    ///                           false = looking for top/left boundary (pane directly in second)
    fn try_move_adjacent_boundary(&mut self, pane_id: PaneId, target_dir: SplitDirection, 
                                   look_for_second_boundary: bool, move_decrease: bool) -> bool {
        match self {
            Layout::Pane(id) => {
                // This is the pane itself - no boundary here
                *id == pane_id && false
            }
            Layout::Split { direction, first, second, ratio } => {
                let first_ids = first.pane_ids();
                let second_ids = second.pane_ids();
                
                if *direction == target_dir {
                    if look_for_second_boundary {
                        // Looking for bottom/right boundary
                        // The pane must be DIRECTLY in first (i.e., first is Pane(pane_id))
                        if let Layout::Pane(id) = first.as_ref() {
                            if *id == pane_id {
                                // Found! This pane is directly adjacent to this boundary
                                if move_decrease {
                                    *ratio = (*ratio - 0.05).clamp(0.1, 0.9);
                                } else {
                                    *ratio = (*ratio + 0.05).clamp(0.1, 0.9);
                                }
                                return true;
                            }
                        }
                    } else {
                        // Looking for top/left boundary
                        // The pane must be DIRECTLY in second (i.e., second is Pane(pane_id))
                        if let Layout::Pane(id) = second.as_ref() {
                            if *id == pane_id {
                                // Found! This pane is directly adjacent to this boundary
                                if move_decrease {
                                    *ratio = (*ratio - 0.05).clamp(0.1, 0.9);
                                } else {
                                    *ratio = (*ratio + 0.05).clamp(0.1, 0.9);
                                }
                                return true;
                            }
                        }
                    }
                }
                
                // Recurse into the child that contains the pane
                if first_ids.contains(&pane_id) {
                    return first.try_move_adjacent_boundary(pane_id, target_dir, look_for_second_boundary, move_decrease);
                }
                if second_ids.contains(&pane_id) {
                    return second.try_move_adjacent_boundary(pane_id, target_dir, look_for_second_boundary, move_decrease);
                }
                
                false
            }
        }
    }

    /// Swap two panes in the layout
    pub fn swap_panes(&mut self, pane_a: PaneId, pane_b: PaneId) {
        self.replace_pane_id(pane_a, 0); // temporary
        self.replace_pane_id(pane_b, pane_a);
        self.replace_pane_id(0, pane_b);
    }

    /// Replace a pane ID with another
    fn replace_pane_id(&mut self, from: PaneId, to: PaneId) {
        match self {
            Layout::Pane(id) => {
                if *id == from {
                    *id = to;
                }
            }
            Layout::Split { first, second, .. } => {
                first.replace_pane_id(from, to);
                second.replace_pane_id(from, to);
            }
        }
    }
}
