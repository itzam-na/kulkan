use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SelectionModifiers {
    pub control: bool,
    pub shift: bool,
}

#[derive(Debug, Default)]
pub struct SelectionState {
    anchor: Option<usize>,
}

impl SelectionState {
    pub fn reset(&mut self) {
        self.anchor = None;
    }

    pub fn select(
        &mut self,
        currently_selected: &BTreeSet<usize>,
        clicked: usize,
        modifiers: SelectionModifiers,
        row_count: usize,
    ) -> BTreeSet<usize> {
        if clicked >= row_count {
            return currently_selected
                .iter()
                .copied()
                .filter(|index| *index < row_count)
                .collect();
        }

        if !modifiers.control && !modifiers.shift {
            self.anchor = Some(clicked);
            if currently_selected.len() == 1 && currently_selected.contains(&clicked) {
                return BTreeSet::new();
            }
            return BTreeSet::from([clicked]);
        }

        if modifiers.control && !modifiers.shift {
            self.anchor = Some(clicked);
            let mut selected = currently_selected.clone();
            if !selected.remove(&clicked) {
                selected.insert(clicked);
            }
            return selected;
        }

        let anchor = if currently_selected.is_empty() {
            self.anchor = Some(clicked);
            clicked
        } else {
            self.anchor
                .filter(|anchor| *anchor < row_count)
                .unwrap_or(clicked)
        };
        let mut selected = if modifiers.control {
            currently_selected.clone()
        } else {
            BTreeSet::new()
        };
        for index in anchor.min(clicked)..=anchor.max(clicked) {
            selected.insert(index);
        }
        selected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_click_selects_only_the_clicked_row() {
        let mut selection = SelectionState::default();

        let selected =
            selection.select(&BTreeSet::from([3, 4]), 1, SelectionModifiers::default(), 8);

        assert_eq!(selected, BTreeSet::from([1]));
    }

    #[test]
    fn plain_click_on_the_sole_selected_row_deselects_it() {
        let mut selection = SelectionState::default();
        let modifiers = SelectionModifiers::default();
        let selected = selection.select(&BTreeSet::new(), 1, modifiers, 8);

        let selected = selection.select(&selected, 1, modifiers, 8);

        assert!(selected.is_empty());
    }

    #[test]
    fn control_click_toggles_one_row_without_clearing_other_selections() {
        let mut selection = SelectionState::default();
        let selected = BTreeSet::from([1, 4]);

        let selected = selection.select(
            &selected,
            2,
            SelectionModifiers {
                control: true,
                shift: false,
            },
            8,
        );

        assert_eq!(selected, BTreeSet::from([1, 2, 4]));

        let selected = selection.select(
            &selected,
            2,
            SelectionModifiers {
                control: true,
                shift: false,
            },
            8,
        );

        assert_eq!(selected, BTreeSet::from([1, 4]));
    }

    #[test]
    fn shift_click_selects_the_inclusive_range_from_the_anchor() {
        let mut selection = SelectionState::default();
        let selected = selection.select(&BTreeSet::new(), 2, SelectionModifiers::default(), 8);

        let selected = selection.select(
            &selected,
            5,
            SelectionModifiers {
                control: false,
                shift: true,
            },
            8,
        );

        assert_eq!(selected, BTreeSet::from([2, 3, 4, 5]));
    }

    #[test]
    fn shift_click_selects_the_range_in_either_direction() {
        let mut selection = SelectionState::default();
        let selected = selection.select(&BTreeSet::new(), 5, SelectionModifiers::default(), 8);

        let selected = selection.select(
            &selected,
            2,
            SelectionModifiers {
                control: false,
                shift: true,
            },
            8,
        );

        assert_eq!(selected, BTreeSet::from([2, 3, 4, 5]));
    }

    #[test]
    fn resetting_selection_starts_a_new_range_at_the_next_click() {
        let mut selection = SelectionState::default();
        let selected = selection.select(&BTreeSet::new(), 2, SelectionModifiers::default(), 8);
        selection.reset();

        let selected = selection.select(
            &selected,
            5,
            SelectionModifiers {
                control: false,
                shift: true,
            },
            8,
        );

        assert_eq!(selected, BTreeSet::from([5]));
    }

    #[test]
    fn control_shift_adds_a_range_to_the_existing_selection() {
        let mut selection = SelectionState::default();
        let selected = selection.select(&BTreeSet::new(), 1, SelectionModifiers::default(), 8);
        let selected = selection.select(
            &selected,
            4,
            SelectionModifiers {
                control: true,
                shift: false,
            },
            8,
        );

        let selected = selection.select(
            &selected,
            6,
            SelectionModifiers {
                control: true,
                shift: true,
            },
            8,
        );

        assert_eq!(selected, BTreeSet::from([1, 4, 5, 6]));
    }
}
