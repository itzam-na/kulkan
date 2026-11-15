use kulkan_library::{
    BrowseItem, BrowsePage, BrowseSelection, BrowseView, CatalogPage, SearchCriteria,
};

#[path = "search_state.rs"]
mod search_state;

pub(crate) use search_state::SearchPanel;
use search_state::SearchWorkspace;

const PAGE_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WorkTicket(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceDestination {
    Browse,
    Search,
}

pub(crate) enum WorkspaceEvent {
    Initialize(BrowseView),
    OpenBrowse(BrowseSelection),
    OpenSearch,
    EditSearch,
    SubmitSearch(SearchCriteria),
    ClearSearch,
    LoadMore,
    LibraryChanged,
    BrowseCompleted {
        ticket: WorkTicket,
        result: Result<BrowsePage, String>,
    },
    SearchCompleted {
        ticket: WorkTicket,
        result: Result<CatalogPage, String>,
    },
}

pub(crate) enum WorkspaceEffect {
    LoadBrowse {
        ticket: WorkTicket,
        selection: BrowseSelection,
        offset: usize,
    },
    Search {
        ticket: WorkTicket,
        criteria: SearchCriteria,
        offset: usize,
    },
    RefreshSearch {
        ticket: WorkTicket,
        criteria: SearchCriteria,
        result_count: usize,
    },
}

pub(crate) struct WorkspaceView<'a> {
    pub destination: WorkspaceDestination,
    pub browse: BrowseWorkspaceView<'a>,
    pub search: SearchWorkspaceView<'a>,
}

pub(crate) struct BrowseWorkspaceView<'a> {
    pub selection: &'a BrowseSelection,
    pub items: &'a [BrowseItem],
    pub next_offset: Option<usize>,
    pub loading: bool,
    pub error: Option<&'a str>,
}

pub(crate) struct SearchWorkspaceView<'a> {
    pub panel: SearchPanel,
    pub criteria: &'a SearchCriteria,
    pub results: Option<&'a CatalogPage>,
    pub pending: bool,
    pub error: Option<&'a str>,
}

struct BrowseState {
    selection: BrowseSelection,
    items: Vec<BrowseItem>,
    next_offset: Option<usize>,
    pending: Option<WorkTicket>,
    error: Option<String>,
}

#[derive(Clone, Copy)]
enum SearchWork {
    Initial,
    More,
    Refresh {
        query_revision: u64,
        refresh_generation: u64,
    },
}

struct PendingSearch {
    ticket: WorkTicket,
    work: SearchWork,
}

pub(crate) struct LibraryWorkspace {
    destination: WorkspaceDestination,
    browse: BrowseState,
    search: SearchWorkspace,
    pending_search: Option<PendingSearch>,
    search_error: Option<String>,
    next_ticket: u64,
    navigation_touched: bool,
}

impl Default for LibraryWorkspace {
    fn default() -> Self {
        Self {
            destination: WorkspaceDestination::Browse,
            browse: BrowseState {
                selection: BrowseSelection::View(BrowseView::Songs),
                items: Vec::new(),
                next_offset: None,
                pending: None,
                error: None,
            },
            search: SearchWorkspace::default(),
            pending_search: None,
            search_error: None,
            next_ticket: 0,
            navigation_touched: false,
        }
    }
}

impl LibraryWorkspace {
    pub(crate) fn dispatch(&mut self, event: WorkspaceEvent) -> Vec<WorkspaceEffect> {
        match event {
            WorkspaceEvent::Initialize(view) => {
                if self.navigation_touched {
                    Vec::new()
                } else {
                    self.open_browse(BrowseSelection::View(view))
                }
            }
            WorkspaceEvent::OpenBrowse(selection) => self.open_browse(selection),
            WorkspaceEvent::OpenSearch => {
                self.navigation_touched = true;
                self.destination = WorkspaceDestination::Search;
                Vec::new()
            }
            WorkspaceEvent::EditSearch => {
                self.navigation_touched = true;
                self.destination = WorkspaceDestination::Search;
                self.search.edit_criteria();
                self.pending_search = None;
                self.search_error = None;
                Vec::new()
            }
            WorkspaceEvent::SubmitSearch(criteria) => self.submit_search(criteria),
            WorkspaceEvent::ClearSearch => {
                self.navigation_touched = true;
                self.destination = WorkspaceDestination::Search;
                self.search.clear();
                self.pending_search = None;
                self.search_error = None;
                Vec::new()
            }
            WorkspaceEvent::LoadMore => self.load_more(),
            WorkspaceEvent::LibraryChanged => self.refresh_library(),
            WorkspaceEvent::BrowseCompleted { ticket, result } => {
                self.complete_browse(ticket, result);
                Vec::new()
            }
            WorkspaceEvent::SearchCompleted { ticket, result } => {
                self.complete_search(ticket, result);
                Vec::new()
            }
        }
    }

    pub(crate) fn view(&self) -> WorkspaceView<'_> {
        WorkspaceView {
            destination: self.destination,
            browse: BrowseWorkspaceView {
                selection: &self.browse.selection,
                items: &self.browse.items,
                next_offset: self.browse.next_offset,
                loading: self.browse.pending.is_some(),
                error: self.browse.error.as_deref(),
            },
            search: SearchWorkspaceView {
                panel: self.search.open(),
                criteria: self.search.criteria(),
                results: self.search.results(),
                pending: self.pending_search.is_some() || self.search.is_search_pending(),
                error: self.search_error.as_deref(),
            },
        }
    }

    fn open_browse(&mut self, selection: BrowseSelection) -> Vec<WorkspaceEffect> {
        self.navigation_touched = true;
        self.destination = WorkspaceDestination::Browse;
        self.browse.selection = selection.clone();
        self.browse.items.clear();
        self.browse.next_offset = None;
        self.browse.error = None;
        let ticket = self.next_ticket();
        self.browse.pending = Some(ticket);
        vec![WorkspaceEffect::LoadBrowse {
            ticket,
            selection,
            offset: 0,
        }]
    }

    fn submit_search(&mut self, criteria: SearchCriteria) -> Vec<WorkspaceEffect> {
        self.destination = WorkspaceDestination::Search;
        let ticket = self.next_ticket();
        self.search.begin_search(criteria.clone());
        self.search_error = None;
        self.pending_search = Some(PendingSearch {
            ticket,
            work: SearchWork::Initial,
        });
        vec![WorkspaceEffect::Search {
            ticket,
            criteria,
            offset: 0,
        }]
    }

    fn load_more(&mut self) -> Vec<WorkspaceEffect> {
        match self.destination {
            WorkspaceDestination::Browse => {
                let Some(offset) = self.browse.next_offset else {
                    return Vec::new();
                };
                if self.browse.pending.is_some() {
                    return Vec::new();
                }
                let ticket = self.next_ticket();
                self.browse.pending = Some(ticket);
                vec![WorkspaceEffect::LoadBrowse {
                    ticket,
                    selection: self.browse.selection.clone(),
                    offset,
                }]
            }
            WorkspaceDestination::Search => {
                if self.pending_search.is_some() {
                    return Vec::new();
                }
                let Some(offset) = self.search.results().and_then(|page| page.next_offset) else {
                    return Vec::new();
                };
                let criteria = self.search.criteria().clone();
                let ticket = self.next_ticket();
                self.pending_search = Some(PendingSearch {
                    ticket,
                    work: SearchWork::More,
                });
                vec![WorkspaceEffect::Search {
                    ticket,
                    criteria,
                    offset,
                }]
            }
        }
    }

    fn refresh_search(&mut self) -> Vec<WorkspaceEffect> {
        if !self.search.has_search() {
            return Vec::new();
        }
        let criteria = self.search.criteria().clone();
        let result_count = self
            .search
            .results()
            .map_or(PAGE_SIZE, |page| page.entries.len().max(PAGE_SIZE));
        let query_revision = self.search.query_revision();
        let refresh_generation = self.search.begin_results_refresh();
        let ticket = self.next_ticket();
        self.pending_search = Some(PendingSearch {
            ticket,
            work: SearchWork::Refresh {
                query_revision,
                refresh_generation,
            },
        });
        vec![WorkspaceEffect::RefreshSearch {
            ticket,
            criteria,
            result_count,
        }]
    }

    fn refresh_library(&mut self) -> Vec<WorkspaceEffect> {
        let mut effects = Vec::new();
        if self.destination == WorkspaceDestination::Browse {
            let selection = self.browse.selection.clone();
            self.browse.items.clear();
            self.browse.next_offset = None;
            self.browse.error = None;
            let ticket = self.next_ticket();
            self.browse.pending = Some(ticket);
            effects.push(WorkspaceEffect::LoadBrowse {
                ticket,
                selection,
                offset: 0,
            });
        }
        effects.extend(self.refresh_search());
        effects
    }

    fn complete_browse(&mut self, ticket: WorkTicket, result: Result<BrowsePage, String>) {
        if self.browse.pending != Some(ticket) {
            return;
        }
        self.browse.pending = None;
        match result {
            Ok(page) => {
                self.browse.items.extend(page.items);
                self.browse.next_offset = page.next_offset;
                self.browse.error = None;
            }
            Err(error) => self.browse.error = Some(error),
        }
    }

    fn complete_search(&mut self, ticket: WorkTicket, result: Result<CatalogPage, String>) {
        let Some(pending) = self.pending_search.take() else {
            return;
        };
        if pending.ticket != ticket {
            self.pending_search = Some(pending);
            return;
        }
        match (pending.work, result) {
            (SearchWork::Initial, Ok(page)) => {
                self.search.submit(self.search.criteria().clone(), page);
                self.search_error = None;
            }
            (SearchWork::More, Ok(page)) => {
                self.search.append_results_page(page);
                self.search_error = None;
            }
            (
                SearchWork::Refresh {
                    query_revision,
                    refresh_generation,
                },
                Ok(page),
            ) => {
                if !self
                    .search
                    .refresh_results(query_revision, refresh_generation, page)
                {
                    return;
                }
                self.search_error = None;
            }
            (SearchWork::Initial, Err(error)) => {
                self.search.fail_search();
                self.search_error = Some(error);
            }
            (SearchWork::More, Err(error)) => self.search_error = Some(error),
            (
                SearchWork::Refresh {
                    query_revision,
                    refresh_generation,
                },
                Err(error),
            ) => {
                if !self
                    .search
                    .fail_results_refresh(query_revision, refresh_generation)
                {
                    return;
                }
                self.search_error = Some(error);
            }
        }
    }

    fn next_ticket(&mut self) -> WorkTicket {
        self.next_ticket = self.next_ticket.wrapping_add(1);
        WorkTicket(self.next_ticket)
    }
}

#[cfg(test)]
mod tests {
    use kulkan_library::{
        BrowseItem, BrowsePage, BrowseSelection, BrowseView, CatalogPage, SearchCriteria,
    };

    use super::{
        LibraryWorkspace, SearchPanel, WorkspaceDestination, WorkspaceEffect, WorkspaceEvent,
    };

    #[test]
    fn opening_a_browse_root_selects_it_and_requests_its_first_page() {
        let mut workspace = LibraryWorkspace::default();
        let selection = BrowseSelection::View(BrowseView::Artists);

        let effects = workspace.dispatch(WorkspaceEvent::OpenBrowse(selection.clone()));

        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            WorkspaceEffect::LoadBrowse { selection: requested, offset: 0, .. }
                if requested == &selection
        ));
        let view = workspace.view();
        assert_eq!(view.destination, WorkspaceDestination::Browse);
        assert_eq!(view.browse.selection, &selection);
        assert!(view.browse.loading);
    }

    #[test]
    fn search_completion_is_cached_without_replacing_the_visible_browse_view() {
        let mut workspace = LibraryWorkspace::default();
        let criteria = SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        };
        let effects = workspace.dispatch(WorkspaceEvent::SubmitSearch(criteria.clone()));
        let ticket = match effects.as_slice() {
            [
                WorkspaceEffect::Search {
                    ticket, offset: 0, ..
                },
            ] => *ticket,
            _ => panic!("expected initial Search work"),
        };
        workspace.dispatch(WorkspaceEvent::OpenBrowse(BrowseSelection::View(
            BrowseView::Songs,
        )));

        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            }),
        });

        let view = workspace.view();
        assert_eq!(view.destination, WorkspaceDestination::Browse);
        assert_eq!(view.search.panel, SearchPanel::Results);
        assert_eq!(view.search.criteria, &criteria);
        assert_eq!(view.search.results.unwrap().next_offset, Some(32));

        workspace.dispatch(WorkspaceEvent::OpenSearch);
        let reopened = workspace.view();
        assert_eq!(reopened.destination, WorkspaceDestination::Search);
        assert_eq!(reopened.search.panel, SearchPanel::Results);
        assert_eq!(reopened.search.criteria, &criteria);
        assert_eq!(reopened.search.results.unwrap().next_offset, Some(32));
    }

    #[test]
    fn reopening_search_after_browse_restores_completed_results() {
        let mut workspace = LibraryWorkspace::default();
        let search = workspace.dispatch(WorkspaceEvent::SubmitSearch(SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        }));
        let search_ticket = match search.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected Search request"),
        };
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: search_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: None,
            }),
        });
        workspace.dispatch(WorkspaceEvent::OpenBrowse(BrowseSelection::View(
            BrowseView::Albums,
        )));
        workspace.dispatch(WorkspaceEvent::OpenSearch);

        let view = workspace.view();
        assert_eq!(view.destination, WorkspaceDestination::Search);
        assert_eq!(view.search.panel, SearchPanel::Results);
        assert!(view.search.results.is_some());
    }

    #[test]
    fn opening_another_browse_view_rejects_the_older_page_completion() {
        let mut workspace = LibraryWorkspace::default();
        let first_effect = workspace.dispatch(WorkspaceEvent::OpenBrowse(BrowseSelection::View(
            BrowseView::Artists,
        )));
        let first_ticket = match first_effect.as_slice() {
            [WorkspaceEffect::LoadBrowse { ticket, .. }] => *ticket,
            _ => panic!("expected first Browse request"),
        };
        let second_effect = workspace.dispatch(WorkspaceEvent::OpenBrowse(BrowseSelection::View(
            BrowseView::Albums,
        )));
        let second_ticket = match second_effect.as_slice() {
            [WorkspaceEffect::LoadBrowse { ticket, .. }] => *ticket,
            _ => panic!("expected second Browse request"),
        };

        workspace.dispatch(WorkspaceEvent::BrowseCompleted {
            ticket: first_ticket,
            result: Ok(BrowsePage {
                items: vec![BrowseItem::Artist {
                    name: Some("stale".to_owned()),
                    track_count: 1,
                }],
                next_offset: None,
            }),
        });

        let view = workspace.view();
        assert_eq!(
            view.browse.selection,
            &BrowseSelection::View(BrowseView::Albums)
        );
        assert!(view.browse.items.is_empty());
        assert!(view.browse.loading);
        assert_ne!(first_ticket, second_ticket);
    }

    #[test]
    fn history_change_refreshes_hidden_search_and_keeps_browse_visible() {
        let mut workspace = LibraryWorkspace::default();
        let criteria = SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        };
        let search_effect = workspace.dispatch(WorkspaceEvent::SubmitSearch(criteria.clone()));
        let search_ticket = match search_effect.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected initial Search request"),
        };
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: search_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            }),
        });
        workspace.dispatch(WorkspaceEvent::OpenBrowse(BrowseSelection::View(
            BrowseView::Songs,
        )));

        let refresh_effect = workspace.dispatch(WorkspaceEvent::LibraryChanged);
        let refresh_ticket = refresh_effect
            .iter()
            .find_map(|effect| match effect {
                WorkspaceEffect::RefreshSearch {
                    ticket,
                    criteria: requested,
                    result_count,
                } => {
                    assert_eq!(requested, &criteria);
                    assert_eq!(*result_count, 32);
                    Some(*ticket)
                }
                WorkspaceEffect::LoadBrowse { .. } | WorkspaceEffect::Search { .. } => None,
            })
            .expect("expected hidden Search refresh");
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: refresh_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: None,
            }),
        });

        let view = workspace.view();
        assert_eq!(view.destination, WorkspaceDestination::Browse);
        assert_eq!(view.search.results.unwrap().next_offset, None);
    }

    #[test]
    fn an_obsolete_search_completion_cannot_replace_a_new_query() {
        let mut workspace = LibraryWorkspace::default();
        let old_effect = workspace.dispatch(WorkspaceEvent::SubmitSearch(SearchCriteria {
            text: Some("old".to_owned()),
            ..SearchCriteria::default()
        }));
        let old_ticket = match old_effect.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected old Search request"),
        };
        let current_criteria = SearchCriteria {
            text: Some("current".to_owned()),
            ..SearchCriteria::default()
        };
        let current_effect =
            workspace.dispatch(WorkspaceEvent::SubmitSearch(current_criteria.clone()));
        let current_ticket = match current_effect.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected current Search request"),
        };

        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: old_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            }),
        });

        let pending_view = workspace.view();
        assert_eq!(pending_view.search.criteria, &current_criteria);
        assert!(pending_view.search.results.is_none());
        assert!(pending_view.search.pending);

        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: current_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: None,
            }),
        });
        assert_eq!(workspace.view().search.results.unwrap().next_offset, None);
    }

    #[test]
    fn clear_search_resets_committed_state_and_pending_work() {
        let mut workspace = LibraryWorkspace::default();
        let effects = workspace.dispatch(WorkspaceEvent::SubmitSearch(SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        }));
        let ticket = match effects.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected Search request"),
        };

        assert!(workspace.dispatch(WorkspaceEvent::ClearSearch).is_empty());
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            }),
        });

        let view = workspace.view();
        assert_eq!(view.destination, WorkspaceDestination::Search);
        assert_eq!(view.search.panel, SearchPanel::Criteria);
        assert_eq!(view.search.criteria, &SearchCriteria::default());
        assert!(view.search.results.is_none());
        assert!(!view.search.pending);
    }

    #[test]
    fn search_request_failure_preserves_criteria_and_returns_to_editable_state() {
        let mut workspace = LibraryWorkspace::default();
        let criteria = SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        };
        let effects = workspace.dispatch(WorkspaceEvent::SubmitSearch(criteria.clone()));
        let ticket = match effects.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected Search request"),
        };

        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket,
            result: Err("server unavailable".to_owned()),
        });

        let view = workspace.view();
        assert_eq!(view.search.panel, SearchPanel::Criteria);
        assert_eq!(view.search.criteria, &criteria);
        assert_eq!(view.search.error, Some("server unavailable"));
        assert!(!view.search.pending);
    }

    #[test]
    fn load_more_requests_and_appends_the_next_search_page() {
        let mut workspace = LibraryWorkspace::default();
        let first = workspace.dispatch(WorkspaceEvent::SubmitSearch(SearchCriteria::default()));
        let first_ticket = match first.as_slice() {
            [
                WorkspaceEffect::Search {
                    ticket, offset: 0, ..
                },
            ] => *ticket,
            _ => panic!("expected first Search page"),
        };
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: first_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            }),
        });

        let more = workspace.dispatch(WorkspaceEvent::LoadMore);
        let more_ticket = match more.as_slice() {
            [
                WorkspaceEffect::Search {
                    ticket, offset: 32, ..
                },
            ] => *ticket,
            _ => panic!("expected next Search page at offset 32"),
        };
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: more_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: None,
            }),
        });

        assert_eq!(workspace.view().search.results.unwrap().next_offset, None);
    }

    #[test]
    fn failed_history_refresh_discards_stale_results_and_keeps_criteria() {
        let mut workspace = LibraryWorkspace::default();
        let criteria = SearchCriteria {
            text: Some("jazz".to_owned()),
            ..SearchCriteria::default()
        };
        let initial = workspace.dispatch(WorkspaceEvent::SubmitSearch(criteria.clone()));
        let initial_ticket = match initial.as_slice() {
            [WorkspaceEffect::Search { ticket, .. }] => *ticket,
            _ => panic!("expected Search request"),
        };
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: initial_ticket,
            result: Ok(CatalogPage {
                entries: Vec::new(),
                next_offset: Some(32),
            }),
        });
        let refresh = workspace.dispatch(WorkspaceEvent::LibraryChanged);
        let refresh_ticket = refresh
            .iter()
            .find_map(|effect| match effect {
                WorkspaceEffect::RefreshSearch { ticket, .. } => Some(*ticket),
                WorkspaceEffect::LoadBrowse { .. } | WorkspaceEffect::Search { .. } => None,
            })
            .expect("expected Search refresh");
        workspace.dispatch(WorkspaceEvent::SearchCompleted {
            ticket: refresh_ticket,
            result: Err("offline".to_owned()),
        });

        let view = workspace.view();
        assert_eq!(view.search.panel, SearchPanel::Criteria);
        assert_eq!(view.search.criteria, &criteria);
        assert!(view.search.results.is_none());
        assert_eq!(view.search.error, Some("offline"));
    }
}
