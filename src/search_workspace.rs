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
    query_revision: u64,
    pending_search: bool,
    results_refresh_generation: u64,
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

    pub(crate) fn begin_search(&mut self, criteria: SearchCriteria) -> u64 {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.criteria = criteria;
        self.results = None;
        self.pending_search = true;
        self.panel = SearchPanel::Results;
        self.invalidate_pending();
        self.generation
    }

    pub(crate) fn query_revision(&self) -> u64 {
        self.query_revision
    }

    pub(crate) fn has_search(&self) -> bool {
        self.pending_search || self.results.is_some()
    }

    pub(crate) fn is_search_pending(&self) -> bool {
        self.pending_search
    }

    pub(crate) fn refresh_results(
        &mut self,
        query_revision: u64,
        refresh_generation: u64,
        page: CatalogPage,
    ) -> bool {
        if self.query_revision != query_revision
            || self.results_refresh_generation != refresh_generation
        {
            return false;
        }
        let search_was_pending = self.pending_search;
        self.results = Some(page);
        self.pending_search = false;
        if search_was_pending {
            self.panel = SearchPanel::Results;
        }
        true
    }

    pub(crate) fn fail_search(&mut self) {
        self.pending_search = false;
        self.panel = SearchPanel::Criteria;
    }

    pub(crate) fn begin_results_refresh(&mut self) -> u64 {
        self.results_refresh_generation = self.results_refresh_generation.wrapping_add(1);
        self.results_refresh_generation
    }

    pub(crate) fn is_current_results_refresh(
        &self,
        query_revision: u64,
        refresh_generation: u64,
    ) -> bool {
        self.query_revision == query_revision
            && self.results_refresh_generation == refresh_generation
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
        self.pending_search = false;
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
        self.pending_search = false;
        self.panel = SearchPanel::Criteria;
    }

    pub(crate) fn clear(&mut self) {
        self.invalidate_pending();
        self.query_revision = self.query_revision.wrapping_add(1);
        self.criteria = SearchCriteria::default();
        self.results = None;
        self.pending_search = false;
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

    #[test]
    fn only_the_latest_history_refresh_updates_the_current_query() {
        let mut workspace = SearchWorkspace::default();
        let query_revision = workspace.begin_search(SearchCriteria::default());
        let earlier_refresh = workspace.begin_results_refresh();
        let latest_refresh = workspace.begin_results_refresh();

        assert!(!workspace.refresh_results(
            query_revision,
            earlier_refresh,
            CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            },
        ));
        assert!(workspace.refresh_results(
            query_revision,
            latest_refresh,
            CatalogPage {
                entries: Vec::new(),
                next_offset: None,
            },
        ));

        let replacement_query = workspace.begin_search(SearchCriteria {
            text: Some("new query".to_owned()),
            ..SearchCriteria::default()
        });
        assert!(!workspace.is_current_results_refresh(query_revision, latest_refresh));
        assert!(workspace.is_current_request(replacement_query));
    }

    #[test]
    fn history_refresh_updates_cached_results_without_leaving_criteria_editing() {
        let mut workspace = SearchWorkspace::default();
        let query_revision = workspace.begin_search(SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        });
        let refresh_generation = workspace.begin_results_refresh();
        workspace.edit_criteria();

        assert!(workspace.refresh_results(
            query_revision,
            refresh_generation,
            CatalogPage {
                entries: Vec::new(),
                next_offset: None,
            },
        ));

        assert_eq!(workspace.open(), SearchPanel::Criteria);
        assert_eq!(workspace.results().unwrap().next_offset, None);
    }

    #[test]
    fn failed_search_returns_to_criteria_instead_of_remaining_pending() {
        let mut workspace = SearchWorkspace::default();
        workspace.begin_search(SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        });

        workspace.fail_search();

        assert_eq!(workspace.open(), SearchPanel::Criteria);
        assert!(!workspace.is_search_pending());
    }
}
