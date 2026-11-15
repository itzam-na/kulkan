use kulkan_library::{CatalogPage, SearchCriteria};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchPanel {
    #[default]
    Criteria,
    Results,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct SearchWorkspace {
    panel: SearchPanel,
    criteria: SearchCriteria,
    results: Option<CatalogPage>,
    generation: u64,
}

impl SearchWorkspace {
    pub(crate) fn begin_request(&mut self) -> u64 {
        self.generation = self.generation.wrapping_add(1);
        self.generation
    }

    pub(crate) fn invalidate_pending(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }

    pub(crate) fn is_current_request(&self, generation: u64) -> bool {
        self.generation == generation
    }

    pub(crate) fn open(&self) -> SearchPanel {
        self.panel
    }

    pub(crate) fn criteria(&self) -> &SearchCriteria {
        &self.criteria
    }

    pub(crate) fn results(&self) -> Option<&CatalogPage> {
        self.results.as_ref()
    }

    pub(crate) fn submit(&mut self, criteria: SearchCriteria, results: CatalogPage) {
        self.criteria = criteria;
        self.results = Some(results);
        self.panel = SearchPanel::Results;
    }

    pub(crate) fn append_results_page(&mut self, page: CatalogPage) {
        if let Some(results) = self.results.as_mut() {
            results.entries.extend(page.entries);
            results.next_offset = page.next_offset;
        }
    }

    pub(crate) fn edit_criteria(&mut self) {
        self.invalidate_pending();
        self.panel = SearchPanel::Criteria;
    }

    pub(crate) fn clear(&mut self) {
        self.invalidate_pending();
        self.criteria = SearchCriteria::default();
        self.results = None;
        self.panel = SearchPanel::Criteria;
    }
}

#[cfg(test)]
mod tests {
    use kulkan_library::{CatalogPage, SearchCriteria};

    use super::{SearchPanel, SearchWorkspace};

    #[test]
    fn search_state_survives_navigation_and_clear_resets_to_blank_criteria() {
        let mut workspace = SearchWorkspace::default();
        let criteria = SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        };
        let results = CatalogPage {
            entries: Vec::new(),
            next_offset: Some(32),
        };

        assert_eq!(workspace.open(), SearchPanel::Criteria);
        workspace.submit(criteria.clone(), results.clone());
        assert_eq!(workspace.open(), SearchPanel::Results);
        assert_eq!(workspace.criteria(), &criteria);
        assert_eq!(workspace.results(), Some(&results));

        workspace.edit_criteria();
        assert_eq!(workspace.open(), SearchPanel::Criteria);
        assert_eq!(workspace.criteria(), &criteria);
        assert_eq!(workspace.results(), Some(&results));

        workspace.clear();
        assert_eq!(workspace.open(), SearchPanel::Criteria);
        assert_eq!(workspace.criteria(), &SearchCriteria::default());
        assert_eq!(workspace.results(), None);
    }

    #[test]
    fn additional_result_pages_remain_cached_while_editing_criteria() {
        let mut workspace = SearchWorkspace::default();
        let criteria = SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        };
        workspace.submit(
            criteria,
            CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            },
        );
        workspace.append_results_page(CatalogPage {
            entries: Vec::new(),
            next_offset: None,
        });
        workspace.edit_criteria();

        assert_eq!(workspace.open(), SearchPanel::Criteria);
        assert_eq!(workspace.results().unwrap().next_offset, None);
    }

    #[test]
    fn newer_search_work_invalidates_older_history_clear_responses() {
        let mut workspace = SearchWorkspace::default();
        let history_clear = workspace.begin_request();
        let newer_search = workspace.begin_request();

        assert!(!workspace.is_current_request(history_clear));
        assert!(workspace.is_current_request(newer_search));

        workspace.invalidate_pending();
        assert!(!workspace.is_current_request(newer_search));
    }
}
