use std::path::PathBuf;

use kulkan_library::{LibraryEntry, TaggedValue, TrackMetadata, present_catalog_entry};

#[test]
fn catalog_presentation_keeps_original_metadata_for_active_entries() {
    let row = present_catalog_entry(&entry(true));

    assert!(row.selectable);
    assert_eq!(row.title.as_deref(), Some("A través del mar"));
    assert_eq!(row.artists, ["Ana & Ben"]);
    assert_eq!(row.release_title.as_deref(), Some("Live set"));
    assert_eq!(row.genres, ["Ambient / electronic"]);
    assert_eq!(row.track_number, None);
    assert_eq!(row.location, PathBuf::from("/music/live.wav"));
}

#[test]
fn inactive_catalog_entries_keep_their_details_but_are_not_selectable() {
    let row = present_catalog_entry(&entry(false));

    assert!(!row.active);
    assert!(!row.selectable);
    assert_eq!(row.title.as_deref(), Some("A través del mar"));
    assert_eq!(row.artists, ["Ana & Ben"]);
}

fn entry(active: bool) -> LibraryEntry {
    LibraryEntry {
        path: PathBuf::from("/music/live.wav"),
        active,
        metadata: TrackMetadata {
            title: Some(tag("A través del mar", "INAM")),
            artists: vec![tag("Ana & Ben", "IART")],
            release_title: Some(tag("Live set", "IPRD")),
            genres: vec![tag("Ambient / electronic", "IGNR")],
            ..TrackMetadata::default()
        },
        duration: None,
    }
}

fn tag(value: &str, field: &str) -> TaggedValue {
    TaggedValue {
        value: value.to_owned(),
        source: "RiffInfo".to_owned(),
        field: field.to_owned(),
    }
}
