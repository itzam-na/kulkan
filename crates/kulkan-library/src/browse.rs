use std::cmp::Ordering;
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::search::{SearchCriteria, search_catalog};
use crate::{Catalog, LibraryEntry};

/// A top-level view in the Library navigation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum BrowseView {
    Artists,
    Albums,
    #[default]
    Songs,
}

/// Identifies one release grouping in the Albums view.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum AlbumKey {
    NoAlbum,
    Release {
        artist_credits: Vec<String>,
        release_title: String,
    },
}

/// Selects a Library browse view or one Artist/Album's tracks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BrowseSelection {
    View(BrowseView),
    Artist(Option<String>),
    Album(AlbumKey),
}

/// One paged row in an Artist, Album, or track browse view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum BrowseItem {
    Artist {
        name: Option<String>,
        track_count: usize,
    },
    Album {
        key: AlbumKey,
        track_count: usize,
    },
    Track(Box<LibraryEntry>),
}

/// A bounded page of Library browse rows.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BrowsePage {
    pub items: Vec<BrowseItem>,
    pub next_offset: Option<usize>,
}

/// Builds a bounded Artist, Album, or track page from the Library catalog.
pub fn browse_catalog(
    catalog: &Catalog,
    selection: BrowseSelection,
    offset: usize,
    limit: usize,
) -> Result<BrowsePage, String> {
    if limit == 0 {
        return Err("browse page size must be greater than zero".to_owned());
    }

    let items = match selection {
        BrowseSelection::View(BrowseView::Artists) => artist_items(catalog),
        BrowseSelection::View(BrowseView::Albums) => album_items(catalog),
        BrowseSelection::View(BrowseView::Songs) => track_items(catalog.entries.clone())?,
        BrowseSelection::Artist(name) => track_items(
            catalog
                .entries
                .iter()
                .filter(|entry| first_track_artist(entry) == name.as_deref())
                .cloned()
                .collect(),
        )?,
        BrowseSelection::Album(key) => track_items(
            catalog
                .entries
                .iter()
                .filter(|entry| album_key(entry) == key)
                .cloned()
                .collect(),
        )?,
    };
    Ok(page(items, offset, limit))
}

fn artist_items(catalog: &Catalog) -> Vec<BrowseItem> {
    let mut counts = HashMap::<Option<String>, usize>::new();
    for entry in &catalog.entries {
        *counts
            .entry(first_track_artist(entry).map(str::to_owned))
            .or_default() += 1;
    }
    let mut artists = counts.into_iter().collect::<Vec<_>>();
    artists
        .sort_by(|(left, _), (right, _)| compare_optional_names(left.as_deref(), right.as_deref()));
    artists
        .into_iter()
        .map(|(name, track_count)| BrowseItem::Artist { name, track_count })
        .collect()
}

fn album_items(catalog: &Catalog) -> Vec<BrowseItem> {
    let mut counts = HashMap::<AlbumKey, usize>::new();
    for entry in &catalog.entries {
        *counts.entry(album_key(entry)).or_default() += 1;
    }
    let mut albums = counts.into_iter().collect::<Vec<_>>();
    albums.sort_by(|(left, _), (right, _)| compare_album_keys(left, right));
    albums
        .into_iter()
        .map(|(key, track_count)| BrowseItem::Album { key, track_count })
        .collect()
}

fn track_items(entries: Vec<LibraryEntry>) -> Result<Vec<BrowseItem>, String> {
    let sorted = search_catalog(
        &Catalog {
            roots: Vec::new(),
            entries,
        },
        &SearchCriteria::default(),
    )?;
    Ok(sorted
        .into_iter()
        .map(|entry| BrowseItem::Track(Box::new(entry)))
        .collect())
}

fn first_track_artist(entry: &LibraryEntry) -> Option<&str> {
    entry
        .metadata
        .artists
        .first()
        .map(|artist| artist.value.trim())
        .filter(|artist| !artist.is_empty())
}

fn album_key(entry: &LibraryEntry) -> AlbumKey {
    let Some(release_title) = entry
        .metadata
        .release_title
        .as_ref()
        .map(|title| title.value.trim())
        .filter(|title| !title.is_empty())
    else {
        return AlbumKey::NoAlbum;
    };
    let mut artist_credits = entry
        .metadata
        .album_artists
        .iter()
        .map(|artist| artist.value.trim())
        .filter(|artist| !artist.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if artist_credits.is_empty()
        && let Some(artist) = first_track_artist(entry)
    {
        artist_credits.push(artist.to_owned());
    }
    AlbumKey::Release {
        artist_credits,
        release_title: release_title.to_owned(),
    }
}

fn compare_album_keys(left: &AlbumKey, right: &AlbumKey) -> Ordering {
    match (left, right) {
        (AlbumKey::NoAlbum, AlbumKey::NoAlbum) => Ordering::Equal,
        (AlbumKey::NoAlbum, _) => Ordering::Greater,
        (_, AlbumKey::NoAlbum) => Ordering::Less,
        (
            AlbumKey::Release {
                artist_credits: left_artists,
                release_title: left_title,
            },
            AlbumKey::Release {
                artist_credits: right_artists,
                release_title: right_title,
            },
        ) => compare_name_lists(left_artists, right_artists)
            .then_with(|| compare_names(left_title, right_title))
            .then_with(|| left_title.cmp(right_title)),
    }
}

fn compare_name_lists(left: &[String], right: &[String]) -> Ordering {
    left.iter()
        .map(|name| name.to_lowercase())
        .cmp(right.iter().map(|name| name.to_lowercase()))
        .then_with(|| left.cmp(right))
}

fn compare_optional_names(left: Option<&str>, right: Option<&str>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => compare_names(left, right).then_with(|| left.cmp(right)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_names(left: &str, right: &str) -> Ordering {
    left.to_lowercase().cmp(&right.to_lowercase())
}

fn page(items: Vec<BrowseItem>, offset: usize, limit: usize) -> BrowsePage {
    let start = offset.min(items.len());
    let end = start.saturating_add(limit).min(items.len());
    BrowsePage {
        items: items[start..end].to_vec(),
        next_offset: (end < items.len()).then_some(end),
    }
}
