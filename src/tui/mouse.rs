use ratatui::layout::Rect;

use crate::tui::app::WorkspaceSection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClickTarget {
    SwitchSection(WorkspaceSection),
    OpenModelPicker,
    ClearQueue,
    ApproveYes,
    ApproveNo,
    QuestionOption(usize),
    QuestionCustom,
    QuestionSubmit,
    QuestionCancel,
    ModelPickerItem(usize),
    SwarmHubCode,
    SwarmHubDefense,
    SwarmHubThreat,
    SwarmHubChat,
    OnboardingConfirm,
    OnboardingCancel,
    OnboardingTelemetry(bool),
    OnboardingFolder(usize),
    OnboardingParent,
    OnboardingToggleManualPath,
    CloseModal,
}

#[derive(Debug, Default, Clone)]
pub struct HitMap {
    regions: Vec<(Rect, ClickTarget)>,
}

impl HitMap {
    pub fn clear(&mut self) {
        self.regions.clear();
    }

    pub fn push(&mut self, rect: Rect, target: ClickTarget) {
        if rect.width > 0 && rect.height > 0 {
            self.regions.push((rect, target));
        }
    }

    /// Returns the top-most target (last recorded) containing `(col, row)`.
    pub fn hit(&self, col: u16, row: u16) -> Option<ClickTarget> {
        self.regions.iter().rev().find_map(|(rect, target)| {
            let inside_x = col >= rect.x && col < rect.x.saturating_add(rect.width);
            let inside_y = row >= rect.y && row < rect.y.saturating_add(rect.height);
            (inside_x && inside_y).then_some(*target)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hit_returns_topmost_overlapping_region() {
        let mut map = HitMap::default();
        map.push(Rect::new(0, 0, 20, 5), ClickTarget::CloseModal);
        map.push(Rect::new(2, 1, 10, 1), ClickTarget::QuestionOption(1));

        assert_eq!(map.hit(5, 1), Some(ClickTarget::QuestionOption(1)));
        assert_eq!(map.hit(0, 0), Some(ClickTarget::CloseModal));
        assert_eq!(map.hit(25, 0), None);
    }
}
