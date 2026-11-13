use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag, TagType};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

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
    /// Stable identity for this path-backed catalog item and its local play history.
    pub id: Uuid,
    /// Canonical path, so overlapping roots and selected directory symlinks deduplicate.
    pub path: PathBuf,
    /// False after a complete scan confirms this path is missing or its root was removed.
    pub active: bool,
    pub metadata: TrackMetadata,
    /// `None` means the metadata reader could not establish a positive duration.
    pub duration: Option<Duration>,
    pub play_count: u64,
    pub content_fingerprint: ContentFingerprint,
}

/// Versioned SHA-256 identity of supported audio content, excluding tags/container bookkeeping.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct ContentFingerprint {
    pub version: u16,
    pub sha256: [u8; 32],
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

    let mut entries = previous.entries.clone();
    let mut discovered = HashSet::new();
    let mut enumerated_directories = HashSet::new();
    let mut excluded_symlink_paths = HashSet::new();
    let mut media_errors = Vec::new();
    let mut active_ids = HashSet::new();
    let mut claimed_ids = HashSet::new();

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
    let scan_data = discovered_paths
        .iter()
        .map(|path| (path.clone(), read_metadata(path)))
        .collect::<Vec<_>>();
    let mut fingerprint_path_counts = HashMap::new();
    for (_, result) in &scan_data {
        if let Ok((_, _, fingerprint)) = result {
            *fingerprint_path_counts
                .entry(fingerprint.clone())
                .or_insert(0) += 1;
        }
    }
    for (path, result) in scan_data {
        match result {
            Ok((metadata, duration, content_fingerprint)) => {
                let same_path = entries.iter().position(|entry| {
                    entry.path == path
                        && entry.content_fingerprint == content_fingerprint
                        && !claimed_ids.contains(&entry.id)
                });
                let previous_same_path = entries.iter().any(|entry| entry.path == path);
                let matching_path = same_path.or_else(|| {
                    if previous_same_path
                        || fingerprint_path_counts.get(&content_fingerprint) != Some(&1)
                    {
                        return None;
                    }
                    let candidates = entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| {
                            entry.path != path
                                && entry.content_fingerprint == content_fingerprint
                                && !claimed_ids.contains(&entry.id)
                                && entry_is_confirmed_missing(
                                    entry,
                                    previous,
                                    &next_roots,
                                    &discovered,
                                    &enumerated_directories,
                                    &excluded_symlink_paths,
                                    &incomplete_paths,
                                )
                        })
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    if candidates.len() == 1 {
                        candidates.first().copied()
                    } else {
                        None
                    }
                });

                if let Some(index) = matching_path {
                    let entry = &mut entries[index];
                    entry.path = path.clone();
                    entry.active = true;
                    entry.metadata = metadata;
                    entry.duration = duration;
                    entry.content_fingerprint = content_fingerprint;
                    active_ids.insert(entry.id);
                    claimed_ids.insert(entry.id);
                } else {
                    let id = Uuid::new_v4();
                    entries.push(LibraryEntry {
                        id,
                        path: path.clone(),
                        active: true,
                        metadata,
                        duration,
                        play_count: 0,
                        content_fingerprint,
                    });
                    active_ids.insert(id);
                    claimed_ids.insert(id);
                }
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

    for entry in &mut entries {
        if active_ids.contains(&entry.id) {
            continue;
        }

        if discovered.contains(&entry.path) {
            // A parse failure is incomplete and must not erase the last usable record.
            if !media_errors.iter().any(|error| error.path == entry.path) {
                entry.active = false;
            }
            continue;
        }

        if entry_is_confirmed_missing(
            entry,
            previous,
            &next_roots,
            &discovered,
            &enumerated_directories,
            &excluded_symlink_paths,
            &incomplete_paths,
        ) {
            entry.active = false;
        }
    }

    let mut incomplete_paths = incomplete_paths.into_iter().collect::<Vec<_>>();
    incomplete_paths.sort();
    media_errors.sort_by(|left, right| left.path.cmp(&right.path));
    entries.sort_by(|left, right| left.path.cmp(&right.path).then(left.id.cmp(&right.id)));

    ScanResult {
        catalog: Catalog {
            roots: next_roots,
            entries,
        },
        incomplete_paths,
        media_errors,
    }
}

fn entry_is_confirmed_missing(
    entry: &LibraryEntry,
    previous: &Catalog,
    roots: &[LibraryRoot],
    discovered: &HashSet<PathBuf>,
    enumerated_directories: &HashSet<PathBuf>,
    excluded_symlink_paths: &HashSet<PathBuf>,
    incomplete_paths: &BTreeSet<PathBuf>,
) -> bool {
    if discovered.contains(&entry.path) {
        return false;
    }
    if !entry.active {
        return true;
    }
    let covered_by_current_root = roots.iter().any(|root| {
        root.resolved_path
            .as_ref()
            .is_some_and(|resolved| entry.path.starts_with(resolved))
    });
    let root_was_removed = !covered_by_current_root
        && previous.roots.iter().any(|old_root| {
            old_root
                .resolved_path
                .as_ref()
                .is_some_and(|resolved| entry.path.starts_with(resolved))
                && !roots.iter().any(|new_root| {
                    new_root.selected_path == old_root.selected_path
                        && new_root.resolved_path == old_root.resolved_path
                })
        });
    if root_was_removed {
        return true;
    }
    let parent = entry.path.parent();
    let incomplete = incomplete_paths.iter().any(|area| {
        entry.path.starts_with(area)
            || parent.is_some_and(|parent| parent == area || parent.starts_with(area))
    });
    !incomplete
        && (parent.is_some_and(|parent| enumerated_directories.contains(parent))
            || excluded_symlink_paths
                .iter()
                .any(|path| entry.path.starts_with(path)))
}

fn previous_resolved_path(previous: &Catalog, selected_path: &Path) -> Option<PathBuf> {
    previous
        .roots
        .iter()
        .find(|root| root.selected_path == selected_path)
        .and_then(|root| root.resolved_path.clone())
}

fn read_metadata(
    path: &Path,
) -> Result<(TrackMetadata, Option<Duration>, ContentFingerprint), String> {
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
    let content_fingerprint = content_fingerprint(path, &extension)?;
    Ok((metadata, duration, content_fingerprint))
}

fn content_fingerprint(path: &Path, extension: &str) -> Result<ContentFingerprint, String> {
    let bytes = fs::read(path).map_err(|error| format!("could not read audio content: {error}"))?;
    let mut hash = Sha256::new();
    hash.update(b"kulkan-audio-content\0");
    hash.update(1_u16.to_be_bytes());
    match extension {
        "wav" => fingerprint_wav(&bytes, &mut hash)?,
        "flac" => fingerprint_flac(&bytes, &mut hash)?,
        "mp3" => fingerprint_mp3(&bytes, &mut hash)?,
        "ogg" | "opus" => fingerprint_ogg(&bytes, &mut hash)?,
        _ => return Err("unsupported audio format for content identity".to_owned()),
    }
    Ok(ContentFingerprint {
        version: 1,
        sha256: hash.finalize().into(),
    })
}

fn fingerprint_wav(bytes: &[u8], hash: &mut Sha256) -> Result<(), String> {
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err("not a RIFF/WAVE file".to_owned());
    }
    let riff_end = 8_usize
        .checked_add(read_u32_le(bytes, 4)? as usize)
        .filter(|end| *end <= bytes.len())
        .ok_or_else(|| "truncated RIFF/WAVE file".to_owned())?;
    let mut offset = 12;
    let mut format = None;
    let mut data_chunks = Vec::new();
    while offset + 8 <= riff_end {
        let id = &bytes[offset..offset + 4];
        let size = read_u32_le(bytes, offset + 4)? as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= riff_end)
            .ok_or_else(|| "truncated RIFF chunk".to_owned())?;
        if id == b"fmt " {
            format = Some(&bytes[start..end]);
        } else if id == b"data" {
            data_chunks.push(&bytes[start..end]);
        }
        offset = end + (size % 2);
    }
    let format = format.ok_or_else(|| "WAVE file has no format chunk".to_owned())?;
    let encoding = read_u16_le(format, 0)?;
    let is_pcm = encoding == 1
        || (encoding == 0xfffe
            && format.len() >= 40
            && read_u16_le(format, 24)? == 1
            && format.get(26..40)
                == Some(&[0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71][..]));
    if !is_pcm {
        return Err("only PCM WAVE files have supported content identity".to_owned());
    }
    if data_chunks.is_empty() {
        return Err("WAVE file has no sample data".to_owned());
    }
    hash.update(b"WAV/PCM\0");
    hash.update((format.len() as u64).to_be_bytes());
    hash.update(format);
    for samples in data_chunks {
        hash.update(samples);
    }
    Ok(())
}

fn fingerprint_flac(bytes: &[u8], hash: &mut Sha256) -> Result<(), String> {
    if bytes.get(..4) != Some(b"fLaC") {
        return Err("not a FLAC stream".to_owned());
    }
    let mut offset = 4;
    let mut first = true;
    let mut stream_info = None;
    loop {
        let header = bytes
            .get(offset..offset + 4)
            .ok_or_else(|| "truncated FLAC metadata".to_owned())?;
        let last = header[0] & 0x80 != 0;
        let block_type = header[0] & 0x7f;
        let size =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        let start = offset + 4;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| "truncated FLAC metadata block".to_owned())?;
        if first {
            if block_type != 0 || size != 34 {
                return Err("FLAC stream does not begin with STREAMINFO".to_owned());
            }
            stream_info = Some(&bytes[start..end]);
            first = false;
        }
        offset = end;
        if last {
            break;
        }
    }
    let info = stream_info.ok_or_else(|| "FLAC stream has no STREAMINFO".to_owned())?;
    let packed = u64::from_be_bytes(info[10..18].try_into().unwrap());
    let sample_rate = packed >> 44;
    let channels = ((packed >> 41) & 0x7) + 1;
    let bits_per_sample = ((packed >> 36) & 0x1f) + 1;
    hash.update(b"FLAC\0");
    hash.update(sample_rate.to_be_bytes());
    hash.update(channels.to_be_bytes());
    hash.update(bits_per_sample.to_be_bytes());
    let audio_end = audio_end_without_trailing_tags(bytes);
    validate_flac_frames(bytes, offset, audio_end)?;
    hash.update(
        bytes
            .get(offset..audio_end)
            .filter(|frames| !frames.is_empty())
            .ok_or_else(|| "FLAC stream has no audio frames".to_owned())?,
    );
    Ok(())
}

fn fingerprint_mp3(bytes: &[u8], hash: &mut Sha256) -> Result<(), String> {
    hash.update(b"MPEG-audio-frames\0");
    let audio_end = audio_end_without_trailing_tags(bytes);
    let mut offset = 0;
    let mut frames = 0_u64;
    while offset < audio_end {
        if let Some(end) = id3_tag_end(bytes, offset) {
            offset = end;
            continue;
        }
        let Some(frame_length) = mp3_frame_length(bytes.get(offset..audio_end).unwrap_or_default())
        else {
            if frames == 0 && bytes[offset..audio_end].iter().all(|byte| *byte == 0) {
                break;
            }
            return Err("invalid or truncated MPEG audio frame".to_owned());
        };
        let frame = bytes
            .get(offset..(offset + frame_length).min(audio_end))
            .ok_or_else(|| "truncated MPEG audio frame".to_owned())?;
        if frame.len() != frame_length {
            return Err("truncated MPEG audio frame".to_owned());
        }
        hash.update((frame.len() as u32).to_be_bytes());
        hash.update(frame);
        frames += 1;
        offset += frame_length;
    }
    if frames == 0 {
        return Err("MPEG audio stream has no frames".to_owned());
    }
    Ok(())
}

fn audio_end_without_trailing_tags(bytes: &[u8]) -> usize {
    let mut end = bytes.len();
    loop {
        let previous_end = end;
        if end >= 128 && bytes.get(end - 128..end - 125) == Some(b"TAG") {
            end -= 128;
        }
        if let Some(tag_start) = apev2_tag_start(bytes, end) {
            end = tag_start;
        }
        if end == previous_end {
            return end;
        }
    }
}

fn validate_flac_frames(bytes: &[u8], start: usize, end: usize) -> Result<(), String> {
    if start >= end || flac_frame_header_end(bytes, start, end).is_none() {
        return Err("FLAC stream has no valid audio frame".to_owned());
    }
    let mut frame_start = start;
    loop {
        let mut search_from = frame_start + 6;
        let next_frame = loop {
            let Some(candidate) = find_flac_frame_header(bytes, search_from, end) else {
                break None;
            };
            if flac_crc16_is_valid(&bytes[frame_start..candidate]) {
                break Some(candidate);
            }
            search_from = candidate + 1;
        };
        if let Some(next_frame) = next_frame {
            frame_start = next_frame;
            continue;
        }
        if flac_crc16_is_valid(&bytes[frame_start..end]) {
            return Ok(());
        }
        return Err("truncated or corrupt FLAC audio frame".to_owned());
    }
}

fn find_flac_frame_header(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    if end < 6 {
        return None;
    }
    (start..=end - 6).find(|&offset| flac_frame_header_end(bytes, offset, end).is_some())
}

fn flac_frame_header_end(bytes: &[u8], start: usize, end: usize) -> Option<usize> {
    let header = bytes.get(start..start + 4)?;
    if header[0] != 0xff
        || header[1] & 0xfc != 0xf8
        || header[1] & 0x02 != 0
        || header[2] >> 4 == 0
        || header[3] >> 4 > 10
        || matches!((header[3] >> 1) & 0x7, 3 | 7)
        || header[3] & 1 != 0
    {
        return None;
    }

    let mut offset = start + 4;
    let first_number_byte = *bytes.get(offset)?;
    let number_length = match first_number_byte {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        0xf8..=0xfb => 5,
        0xfc..=0xfd => 6,
        0xfe => 7,
        _ => return None,
    };
    for continuation in bytes.get(offset + 1..offset + number_length)? {
        if continuation & 0xc0 != 0x80 {
            return None;
        }
    }
    offset += number_length;

    match header[2] >> 4 {
        6 => offset += 1,
        7 => offset += 2,
        _ => {}
    }
    match header[2] & 0x0f {
        12 => offset += 1,
        13 | 14 => offset += 2,
        15 => return None,
        _ => {}
    }
    let crc_offset = offset;
    let crc = *bytes.get(crc_offset)?;
    if crc8(&bytes[start..crc_offset]) != crc || crc_offset + 1 > end {
        return None;
    }
    Some(crc_offset + 1)
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0_u8;
    for byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x07
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn flac_crc16_is_valid(frame: &[u8]) -> bool {
    if frame.len() < 2 {
        return false;
    }
    let checksum_offset = frame.len() - 2;
    let expected = u16::from_be_bytes([frame[checksum_offset], frame[checksum_offset + 1]]);
    let mut crc = 0_u16;
    for byte in &frame[..checksum_offset] {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x8005
            } else {
                crc << 1
            };
        }
    }
    crc == expected
}

fn apev2_tag_start(bytes: &[u8], end: usize) -> Option<usize> {
    let footer_start = end.checked_sub(32)?;
    let footer = bytes.get(footer_start..end)?;
    if &footer[..8] != b"APETAGEX" {
        return None;
    }
    let size = u32::from_le_bytes(footer[12..16].try_into().ok()?) as usize;
    let tag_start = end.checked_sub(size)?;
    let has_header = u32::from_le_bytes(footer[20..24].try_into().ok()?) & 0x2000_0000 != 0;
    let tag_start = if has_header {
        tag_start.checked_sub(32)?
    } else {
        tag_start
    };
    (tag_start <= footer_start).then_some(tag_start)
}

fn id3_tag_end(bytes: &[u8], offset: usize) -> Option<usize> {
    let header = bytes.get(offset..offset + 10)?;
    if &header[..3] != b"ID3" || header[6..10].iter().any(|byte| byte & 0x80 != 0) {
        return None;
    }
    let size = (usize::from(header[6]) << 21)
        | (usize::from(header[7]) << 14)
        | (usize::from(header[8]) << 7)
        | usize::from(header[9]);
    offset
        .checked_add(10 + size + usize::from(header[5] & 0x10 != 0) * 10)
        .filter(|end| *end <= bytes.len())
}

fn mp3_frame_length(frame: &[u8]) -> Option<usize> {
    let header = u32::from_be_bytes(frame.get(..4)?.try_into().ok()?);
    if header >> 21 != 0x7ff {
        return None;
    }
    let version = (header >> 19) & 0x3;
    let layer = (header >> 17) & 0x3;
    let bitrate_index = ((header >> 12) & 0xf) as usize;
    let sample_index = ((header >> 10) & 0x3) as usize;
    if version == 1 || layer != 1 || !(1..=14).contains(&bitrate_index) || sample_index == 3 {
        return None;
    }
    const MPEG1_LAYER3: [u16; 16] = [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 0,
    ];
    const MPEG2_LAYER3: [u16; 16] = [
        0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160, 0,
    ];
    const SAMPLE_RATES: [u32; 3] = [44_100, 48_000, 32_000];
    let divisor = match version {
        3 => 1,
        2 => 2,
        _ => 4,
    };
    let sample_rate = SAMPLE_RATES[sample_index] / divisor;
    let bitrate = if version == 3 {
        MPEG1_LAYER3[bitrate_index]
    } else {
        MPEG2_LAYER3[bitrate_index]
    };
    let padding = ((header >> 9) & 1) as usize;
    Some(
        (if version == 3 { 144 } else { 72 } * usize::from(bitrate) * 1000) / sample_rate as usize
            + padding,
    )
}

fn fingerprint_ogg(bytes: &[u8], hash: &mut Sha256) -> Result<(), String> {
    let mut offset = 0;
    let mut serial = None;
    let mut expected_sequence = None;
    let mut saw_eos = false;
    let mut packet_buffer = Vec::new();
    let mut packets = Vec::new();
    while offset < bytes.len() {
        let header = bytes
            .get(offset..offset + 27)
            .filter(|header| &header[..4] == b"OggS" && header[4] == 0)
            .ok_or_else(|| "invalid or truncated Ogg page".to_owned())?;
        let page_serial = u32::from_le_bytes(header[14..18].try_into().unwrap());
        if saw_eos {
            return Err("Ogg stream contains pages after its end marker".to_owned());
        }
        if serial.is_some_and(|serial| serial != page_serial) {
            return Err("multiple Ogg logical streams are not supported".to_owned());
        }
        serial = Some(page_serial);
        let sequence = u32::from_le_bytes(header[18..22].try_into().unwrap());
        if expected_sequence.is_some_and(|expected| sequence != expected) {
            return Err("Ogg page sequence is incomplete".to_owned());
        }
        expected_sequence = Some(sequence.wrapping_add(1));
        saw_eos = header[5] & 0x04 != 0;
        let segment_count = header[26] as usize;
        let lacing = bytes
            .get(offset + 27..offset + 27 + segment_count)
            .ok_or_else(|| "truncated Ogg lacing table".to_owned())?;
        let mut body_offset = offset + 27 + segment_count;
        for &segment_size in lacing {
            let end = body_offset + segment_size as usize;
            packet_buffer.extend_from_slice(
                bytes
                    .get(body_offset..end)
                    .ok_or_else(|| "truncated Ogg page body".to_owned())?,
            );
            body_offset = end;
            if segment_size < 255 {
                packets.push(std::mem::take(&mut packet_buffer));
            }
        }
        offset = body_offset;
    }
    if !saw_eos {
        return Err("Ogg stream has no end marker".to_owned());
    }
    if !packet_buffer.is_empty() {
        return Err("Ogg stream ends with an incomplete packet".to_owned());
    }
    let Some(first) = packets.first() else {
        return Err("Ogg stream has no packets".to_owned());
    };
    if first.get(..7) == Some(&b"\x01vorbis"[..]) {
        if packets.len() < 4 || packets[1].first() != Some(&3) || packets[2].first() != Some(&5) {
            return Err("incomplete Ogg Vorbis headers".to_owned());
        }
        hash.update(b"Ogg/Vorbis\0");
        hash_packet(hash, &packets[0]);
        hash_packet(hash, &packets[2]);
        for packet in &packets[3..] {
            hash_packet(hash, packet);
        }
    } else if first.get(..8) == Some(b"OpusHead") {
        if packets.len() < 3 || packets[1].get(..8) != Some(b"OpusTags") {
            return Err("incomplete Ogg Opus headers".to_owned());
        }
        let head = &packets[0];
        if head.len() < 19 {
            return Err("invalid OpusHead packet".to_owned());
        }
        hash.update(b"Ogg/Opus\0");
        hash_packet(hash, head);
        for packet in &packets[2..] {
            hash_packet(hash, packet);
        }
    } else {
        return Err("unsupported Ogg codec".to_owned());
    }
    Ok(())
}

fn hash_packet(hash: &mut Sha256, packet: &[u8]) {
    hash.update((packet.len() as u64).to_be_bytes());
    hash.update(packet);
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .map(|value| u16::from_le_bytes(value.try_into().unwrap()))
        .ok_or_else(|| "truncated audio header".to_owned())
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .map(|value| u32::from_le_bytes(value.try_into().unwrap()))
        .ok_or_else(|| "truncated audio header".to_owned())
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
