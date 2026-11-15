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
    query_revision: u64,
    pending_search: bool,
    results_refresh_generation: u64,
}

impl SearchWorkspace {
    pub(crate) fn begin_search(&mut self, criteria: SearchCriteria) {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.criteria = criteria;
        self.results = None;
        self.pending_search = true;
        self.panel = SearchPanel::Results;
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

    pub(crate) fn fail_results_refresh(
        &mut self,
        query_revision: u64,
        refresh_generation: u64,
    ) -> bool {
        if !self.is_current_results_refresh(query_revision, refresh_generation) {
            return false;
        }
        self.results = None;
        self.pending_search = false;
        self.panel = SearchPanel::Criteria;
        true
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
        self.pending_search = false;
        self.panel = SearchPanel::Criteria;
    }

    pub(crate) fn clear(&mut self) {
        self.query_revision = self.query_revision.wrapping_add(1);
        self.criteria = SearchCriteria::default();
        self.results = None;
        self.pending_search = false;
        self.panel = SearchPanel::Criteria;
    }
}
