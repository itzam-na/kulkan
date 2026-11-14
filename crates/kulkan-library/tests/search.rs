use std::path::PathBuf;
use std::time::Duration;

use kulkan_library::{
    Catalog, ContentFingerprint, LibraryEntry, NumericFilter, SearchCriteria, TaggedValue,
    TextFilter, TrackMetadata, search_catalog,
};
use uuid::Uuid;

#[test]
fn library_search_combines_criteria_and_ignores_case_and_diacritics() {
    let catalog = catalog();
    let results = search_catalog(
        &catalog,
        &SearchCriteria {
            text: Some("angela".to_owned()),
            artist: Some(TextFilter::Contains("maria".to_owned())),
            not_yet_played: Some(true),
            ..SearchCriteria::default()
        },
    )
    .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path, PathBuf::from("/music/angela.mp3"));
}

#[test]
fn missing_values_and_inclusive_numeric_ranges_are_explicit() {
    let catalog = catalog();
    let missing_year = search_catalog(
        &catalog,
        &SearchCriteria {
            release_year: Some(NumericFilter::IsMissing),
            genre: Some(TextFilter::IsMissing),
            play_count: Some(NumericFilter::RangeInclusive { min: 2, max: 4 }),
            ..SearchCriteria::default()
        },
    )
    .unwrap();
    assert_eq!(missing_year.len(), 1);
    assert_eq!(missing_year[0].path, PathBuf::from("/music/untagged.wav"));

    let duration = search_catalog(
        &catalog,
        &SearchCriteria {
            duration_seconds: Some(NumericFilter::RangeInclusive { min: 180, max: 180 }),
            ..SearchCriteria::default()
        },
    )
    .unwrap();
    assert_eq!(duration.len(), 1);
    assert_eq!(duration[0].path, PathBuf::from("/music/angela.mp3"));
}

#[test]
fn results_sort_by_artist_release_track_then_title_with_missing_values_last() {
    let results = search_catalog(&catalog(), &SearchCriteria::default()).unwrap();
    let paths = results
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        vec![
            PathBuf::from("/music/angela.mp3"),
            PathBuf::from("/music/other.mp3"),
            PathBuf::from("/music/untagged.wav"),
        ]
    );
}

#[test]
fn release_date_precedes_track_number_within_an_artist() {
    let mut early = entry(
        "/music/early.wav",
        metadata(Some("Artist"), Some("Album"), Some(1980), Some(2), &[]),
        Some(Duration::from_secs(60)),
        0,
    );
    early.metadata.release_date = Some(tag("1980-01-01"));
    let mut late = entry(
        "/music/late.wav",
        metadata(Some("Artist"), Some("Album"), Some(1980), Some(1), &[]),
        Some(Duration::from_secs(60)),
        0,
    );
    late.metadata.release_date = Some(tag("1980-12-01"));
    let results = search_catalog(
        &Catalog {
            roots: Vec::new(),
            entries: vec![late, early],
        },
        &SearchCriteria::default(),
    )
    .unwrap();

    assert_eq!(results[0].path, PathBuf::from("/music/early.wav"));
    assert_eq!(results[1].path, PathBuf::from("/music/late.wav"));
}

fn catalog() -> Catalog {
    Catalog {
        roots: Vec::new(),
        entries: vec![
            entry(
                "/music/angela.mp3",
                metadata(
                    Some("Ângela Maria"),
                    Some("Canções"),
                    Some(1980),
                    Some(1),
                    &["MPB"],
                ),
                Some(Duration::from_secs(180)),
                0,
            ),
            entry(
                "/music/other.mp3",
                metadata(Some("Beto"), Some("Álbum"), Some(2000), Some(2), &["Rock"]),
                Some(Duration::from_secs(240)),
                1,
            ),
            entry(
                "/music/untagged.wav",
                metadata(None, None, None, None, &[]),
                None,
                3,
            ),
        ],
    }
}

fn entry(
    path: &str,
    metadata: TrackMetadata,
    duration: Option<Duration>,
    play_count: u64,
) -> LibraryEntry {
    LibraryEntry {
        id: Uuid::nil(),
        path: PathBuf::from(path),
        active: true,
        metadata,
        duration,
        play_count,
        content_fingerprint: ContentFingerprint {
            version: 1,
            sha256: [0; 32],
        },
    }
}

fn metadata(
    artist: Option<&str>,
    album: Option<&str>,
    year: Option<u16>,
    track_number: Option<u32>,
    genres: &[&str],
) -> TrackMetadata {
    TrackMetadata {
        artists: artist.map(tag).into_iter().collect(),
        release_title: album.map(tag),
        release_year: year,
        track_number,
        genres: genres.iter().map(|genre| tag(genre)).collect(),
        ..TrackMetadata::default()
    }
}

fn tag(value: &str) -> TaggedValue {
    TaggedValue {
        value: value.to_owned(),
        source: "test".to_owned(),
        field: "test".to_owned(),
    }
}
