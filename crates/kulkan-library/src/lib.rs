use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag, TagType};
use serde::{Deserialize, Serialize};

/// One original metadata value and the format field it came from.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TaggedValue {
    pub value: String,
    pub source: String,
    pub field: String,
}

/// Original display metadata, kept separate from later search-normalized values.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrackMetadata {
    pub title: Option<TaggedValue>,
    pub artists: Vec<TaggedValue>,
    pub album_artists: Vec<TaggedValue>,
    pub release_title: Option<TaggedValue>,
    pub release_date: Option<TaggedValue>,
    pub release_year: Option<u16>,
    pub track_number: Option<u32>,
    pub track_total: Option<u32>,
    pub genres: Vec<TaggedValue>,
    /// Other non-empty values for fields with a deterministic source precedence.
    pub conflicting_values: Vec<TaggedValue>,
}

/// A directory the listener explicitly selected and its resolved directory, when available.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryRoot {
    pub selected_path: PathBuf,
    pub resolved_path: Option<PathBuf>,
}

/// One file-backed item in the local catalog.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryEntry {
    /// Canonical path, so overlapping roots and selected directory symlinks deduplicate.
    pub path: PathBuf,
    /// False after a complete scan confirms this path is missing or its root was removed.
    pub active: bool,
    pub metadata: TrackMetadata,
    /// `None` means the metadata reader could not establish a positive duration.
    pub duration: Option<Duration>,
}

/// The last known library roots and entries. A scan uses this snapshot to preserve history rows.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Catalog {
    pub roots: Vec<LibraryRoot>,
    pub entries: Vec<LibraryEntry>,
}

/// Results from an explicit library scan.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScanResult {
    pub catalog: Catalog,
    /// Paths whose contents could not be completely enumerated. Their last-known entries remain.
    pub incomplete_paths: Vec<PathBuf>,
    /// Supported audio files that could not be read as valid media.
    pub media_errors: Vec<MediaError>,
}

/// An actionable error associated with a discovered audio file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MediaError {
    pub path: PathBuf,
    pub message: String,
}

/// Small result returned after changing roots; adding a root never scans it.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct LibraryState {
    pub selected_roots: Vec<PathBuf>,
    pub catalog_entries: usize,
}

/// Summary of an explicit scan, without transferring the full catalog in one IPC frame.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ScanSummary {
    pub incomplete_paths: Vec<PathBuf>,
    pub media_errors: Vec<MediaError>,
    pub active_entries: usize,
    pub inactive_entries: usize,
}

/// One bounded page of catalog entries for UI browsing.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogPage {
    pub entries: Vec<LibraryEntry>,
    pub next_offset: Option<usize>,
}

/// UI-ready catalog fields, retaining the displayed tag text and inactive state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogRow {
    pub location: PathBuf,
    pub title: Option<String>,
    pub artists: Vec<String>,
    pub release_title: Option<String>,
    pub release_year: Option<u16>,
    pub track_number: Option<u32>,
    pub genres: Vec<String>,
    pub duration: Option<Duration>,
    pub active: bool,
    pub selectable: bool,
}

/// Creates a presentation row without normalizing or inventing metadata.
pub fn present_catalog_entry(entry: &LibraryEntry) -> CatalogRow {
    CatalogRow {
        location: entry.path.clone(),
        title: entry.metadata.title.as_ref().map(|tag| tag.value.clone()),
        artists: entry
            .metadata
            .artists
            .iter()
            .map(|tag| tag.value.clone())
            .collect(),
        release_title: entry
            .metadata
            .release_title
            .as_ref()
            .map(|tag| tag.value.clone()),
        release_year: entry.metadata.release_year,
        track_number: entry.metadata.track_number,
        genres: entry
            .metadata
            .genres
            .iter()
            .map(|tag| tag.value.clone())
            .collect(),
        duration: entry.duration,
        active: entry.active,
        selectable: entry.active,
    }
}

/// Scans the selected roots without following nested symlinks or modifying media files.
///
/// The prior catalog is retained when a selected root becomes unavailable. An entry becomes
/// inactive only if its containing directory was successfully enumerated or its root was removed.
pub fn scan_library(previous: &Catalog, roots: &[PathBuf]) -> ScanResult {
    let mut next_roots = Vec::new();
    let mut resolved_roots = Vec::new();
    let mut incomplete_paths = BTreeSet::new();

    for selected_path in roots {
        let selected_path = absolute_path(selected_path);
        if next_roots
            .iter()
            .any(|root: &LibraryRoot| root.selected_path == selected_path)
        {
            continue;
        }

        match fs::canonicalize(&selected_path) {
            Ok(resolved_path) if resolved_path.is_dir() => {
                if !resolved_roots.contains(&resolved_path) {
                    resolved_roots.push(resolved_path.clone());
                }
                next_roots.push(LibraryRoot {
                    selected_path,
                    resolved_path: Some(resolved_path),
                });
            }
            Ok(_) => {
                incomplete_paths.insert(selected_path.clone());
                next_roots.push(LibraryRoot {
                    resolved_path: previous_resolved_path(previous, &selected_path),
                    selected_path,
                });
            }
            Err(_) => {
                incomplete_paths.insert(selected_path.clone());
                next_roots.push(LibraryRoot {
                    resolved_path: previous_resolved_path(previous, &selected_path),
                    selected_path,
                });
            }
        }
    }

    let mut entries_by_path = previous
        .entries
        .iter()
        .cloned()
        .map(|entry| (entry.path.clone(), entry))
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut discovered = HashSet::new();
    let mut enumerated_directories = HashSet::new();
    let mut excluded_symlink_paths = HashSet::new();
    let mut media_errors = Vec::new();

    for root in &resolved_roots {
        enumerate_directory(
            root,
            root,
            &mut discovered,
            &mut enumerated_directories,
            &mut excluded_symlink_paths,
            &mut incomplete_paths,
        );
    }

    let mut discovered_paths = discovered.iter().cloned().collect::<Vec<_>>();
    discovered_paths.sort();
    for path in &discovered_paths {
        match read_metadata(path) {
            Ok((metadata, duration)) => {
                entries_by_path.insert(
                    path.clone(),
                    LibraryEntry {
                        path: path.clone(),
                        active: true,
                        metadata,
                        duration,
                    },
                );
            }
            Err(error) => {
                incomplete_paths.insert(path.clone());
                media_errors.push(MediaError {
                    path: path.clone(),
                    message: error,
                });
            }
        }
    }

    for entry in entries_by_path.values_mut() {
        if discovered.contains(&entry.path) {
            entry.active = true;
            continue;
        }

        let covered_by_current_root = next_roots.iter().any(|root| {
            root.resolved_path
                .as_ref()
                .is_some_and(|resolved| entry.path.starts_with(resolved))
        });
        let selected_root_was_removed = !covered_by_current_root
            && previous.roots.iter().any(|old_root| {
                old_root
                    .resolved_path
                    .as_ref()
                    .is_some_and(|resolved| entry.path.starts_with(resolved))
                    && !next_roots.iter().any(|new_root| {
                        new_root.selected_path == old_root.selected_path
                            && new_root.resolved_path == old_root.resolved_path
                    })
            });
        if selected_root_was_removed {
            entry.active = false;
            continue;
        }

        let parent = entry.path.parent();
        let area_is_incomplete = incomplete_paths.iter().any(|area| {
            entry.path.starts_with(area)
                || parent.is_some_and(|parent| parent == area || parent.starts_with(area))
        });
        if !area_is_incomplete
            && excluded_symlink_paths
                .iter()
                .any(|path| entry.path.starts_with(path))
        {
            entry.active = false;
            continue;
        }
        if !area_is_incomplete
            && parent.is_some_and(|parent| enumerated_directories.contains(parent))
        {
            entry.active = false;
        }
    }

    let mut incomplete_paths = incomplete_paths.into_iter().collect::<Vec<_>>();
    incomplete_paths.sort();
    media_errors.sort_by(|left, right| left.path.cmp(&right.path));
    let mut entries = entries_by_path.into_values().collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));

    ScanResult {
        catalog: Catalog {
            roots: next_roots,
            entries,
        },
        incomplete_paths,
        media_errors,
    }
}

fn previous_resolved_path(previous: &Catalog, selected_path: &Path) -> Option<PathBuf> {
    previous
        .roots
        .iter()
        .find(|root| root.selected_path == selected_path)
        .and_then(|root| root.resolved_path.clone())
}

fn read_metadata(path: &Path) -> Result<(TrackMetadata, Option<Duration>), String> {
    let tagged_file = lofty::read_from_path(path)
        .map_err(|error| format!("could not read audio metadata: {error}"))?;
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let tags = preferred_tags(&tagged_file, &extension);
    let metadata = metadata_from_tags(&tags, &extension);
    let duration = tagged_file.properties().duration();
    let duration = (!duration.is_zero()).then_some(duration);
    Ok((metadata, duration))
}

fn preferred_tags<'a>(tagged_file: &'a lofty::file::TaggedFile, extension: &str) -> Vec<&'a Tag> {
    let order: &[TagType] = match extension {
        "mp3" => &[TagType::Id3v2, TagType::Id3v1],
        "wav" => &[TagType::Id3v2, TagType::RiffInfo, TagType::Id3v1],
        "flac" | "ogg" | "opus" => &[TagType::VorbisComments],
        _ => &[],
    };
    order
        .iter()
        .filter_map(|tag_type| tagged_file.tag(*tag_type))
        .collect()
}

fn metadata_from_tags(tags: &[&Tag], extension: &str) -> TrackMetadata {
    let mut metadata = TrackMetadata::default();
    let mut genre_tags = tags.to_vec();
    if extension == "wav" {
        genre_tags.sort_by_key(|tag| match tag.tag_type() {
            TagType::RiffInfo => 0,
            TagType::Id3v2 => 1,
            TagType::Id3v1 => 2,
            _ => 3,
        });
    }
    metadata.title = first_value(tags, ItemKey::TrackTitle, &mut metadata.conflicting_values);
    metadata.artists = first_values(tags, ItemKey::TrackArtist, &mut metadata.conflicting_values);
    deduplicate_exact_values(&mut metadata.artists);
    metadata.album_artists =
        first_values(tags, ItemKey::AlbumArtist, &mut metadata.conflicting_values);
    deduplicate_exact_values(&mut metadata.album_artists);
    metadata.release_title =
        first_value(tags, ItemKey::AlbumTitle, &mut metadata.conflicting_values);
    metadata.release_date = first_release_date(tags, &mut metadata.conflicting_values);
    metadata.release_year = metadata
        .release_date
        .as_ref()
        .and_then(|date| parse_release_year(&date.value));
    let (track_number, track_total) = first_track_position(tags, &mut metadata.conflicting_values);
    metadata.track_number = track_number;
    metadata.track_total = track_total;
    metadata.genres = first_values(
        &genre_tags,
        ItemKey::Genre,
        &mut metadata.conflicting_values,
    );
    metadata
}

fn deduplicate_exact_values(values: &mut Vec<TaggedValue>) {
    let mut seen = HashSet::new();
    values.retain(|value| seen.insert(value.value.clone()));
}

fn first_value(
    tags: &[&Tag],
    key: ItemKey,
    conflicts: &mut Vec<TaggedValue>,
) -> Option<TaggedValue> {
    let values = first_values(tags, key, conflicts);
    if values.len() > 1 {
        conflicts.extend(values.iter().skip(1).cloned());
    }
    values.into_iter().next()
}

fn first_release_date(tags: &[&Tag], conflicts: &mut Vec<TaggedValue>) -> Option<TaggedValue> {
    let mut selected = None;
    for tag in tags {
        let values = tagged_values(tag, ItemKey::ReleaseDate);
        if selected.is_some() || values.is_empty() {
            conflicts.extend(values);
            continue;
        }

        if let Some(index) = values
            .iter()
            .position(|value| parse_release_year(&value.value).is_some())
        {
            selected = Some(values[index].clone());
            conflicts.extend(
                values
                    .into_iter()
                    .enumerate()
                    .filter_map(|(value_index, value)| (value_index != index).then_some(value)),
            );
        } else {
            conflicts.extend(values);
        }
    }
    selected
}

fn parse_release_year(value: &str) -> Option<u16> {
    let mut parts = value.split('-');
    let year_text = parts.next()?;
    if year_text.len() != 4 || !year_text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year = year_text.parse::<u16>().ok()?;
    if year == 0 {
        return None;
    }

    let month = parts.next();
    let day = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let Some(month) = month else {
        return Some(year);
    };
    if month.len() != 2 || !month.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let month = month.parse::<u8>().ok()?;
    if !(1..=12).contains(&month) {
        return None;
    }
    let Some(day) = day else {
        return Some(year);
    };
    if day.len() != 2 || !day.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let day = day.parse::<u8>().ok()?;
    let last_day = match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    (1..=last_day).contains(&day).then_some(year)
}

#[allow(clippy::manual_is_multiple_of)]
fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn first_values(tags: &[&Tag], key: ItemKey, conflicts: &mut Vec<TaggedValue>) -> Vec<TaggedValue> {
    let Some((primary_tag, primary_values)) = tags.iter().find_map(|tag| {
        let values = tagged_values(tag, key);
        (!values.is_empty()).then_some((*tag, values))
    }) else {
        return Vec::new();
    };

    for tag in tags {
        if tag.tag_type() == primary_tag.tag_type() {
            continue;
        }
        conflicts.extend(tagged_values(tag, key));
    }
    primary_values
}

fn tagged_values(tag: &Tag, key: ItemKey) -> Vec<TaggedValue> {
    let source = format!("{:?}", tag.tag_type());
    let field = key
        .map_key(tag.tag_type())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{key:?}"));
    tag.get_strings(key)
        .filter(|value| !value.trim().is_empty())
        .map(|value| TaggedValue {
            value: value.to_owned(),
            source: source.clone(),
            field: field.clone(),
        })
        .collect()
}

fn first_track_position(
    tags: &[&Tag],
    conflicts: &mut Vec<TaggedValue>,
) -> (Option<u32>, Option<u32>) {
    let values = first_values(tags, ItemKey::TrackNumber, conflicts);
    let Some(value) = values.first() else {
        return (None, None);
    };
    let (number, total) = value.value.split_once('/').unwrap_or((&value.value, ""));
    let number = number
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|number| *number > 0);
    let total = total
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|total| *total > 0)
        .or_else(|| {
            tags.iter()
                .find_map(|tag| tag.get_string(ItemKey::TrackTotal))
                .and_then(|total| total.trim().parse::<u32>().ok())
                .filter(|total| *total > 0)
        });
    (number, total)
}

fn enumerate_directory(
    root: &Path,
    directory: &Path,
    discovered: &mut HashSet<PathBuf>,
    enumerated_directories: &mut HashSet<PathBuf>,
    excluded_symlink_paths: &mut HashSet<PathBuf>,
    incomplete_paths: &mut BTreeSet<PathBuf>,
) {
    let resolved_directory = match fs::canonicalize(directory) {
        Ok(path) => path,
        Err(_) => {
            incomplete_paths.insert(directory.to_path_buf());
            return;
        }
    };
    if !resolved_directory.starts_with(root) {
        incomplete_paths.insert(directory.to_path_buf());
        return;
    }
    let entries = match fs::read_dir(&resolved_directory) {
        Ok(entries) => entries,
        Err(_) => {
            incomplete_paths.insert(resolved_directory);
            return;
        }
    };
    enumerated_directories.insert(resolved_directory.clone());

    for item in entries {
        let item = match item {
            Ok(item) => item,
            Err(_) => {
                incomplete_paths.insert(resolved_directory.clone());
                continue;
            }
        };
        let path = item.path();
        let file_type = match item.file_type() {
            Ok(file_type) => file_type,
            Err(_) => {
                incomplete_paths.insert(path);
                continue;
            }
        };

        if file_type.is_symlink() {
            excluded_symlink_paths.insert(path);
            continue;
        }
        if file_type.is_dir() {
            enumerate_directory(
                root,
                &path,
                discovered,
                enumerated_directories,
                excluded_symlink_paths,
                incomplete_paths,
            );
        } else if file_type.is_file() && is_supported_audio(&path) {
            match fs::canonicalize(&path) {
                Ok(resolved_path) if resolved_path.starts_with(root) => {
                    discovered.insert(resolved_path);
                }
                Ok(_) => {
                    incomplete_paths.insert(path);
                }
                Err(_) => {
                    incomplete_paths.insert(path);
                }
            }
        }
    }
}

fn is_supported_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "mp3" | "wav" | "flac" | "ogg" | "opus"
            )
        })
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    }
}
