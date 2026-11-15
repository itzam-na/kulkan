use std::path::PathBuf;
use std::time::Duration;

use kulkan_library::{
    AlbumKey, BrowseItem, BrowseSelection, BrowseView, Catalog, ContentFingerprint, LibraryEntry,
    TaggedValue, TrackMetadata, browse_catalog,
};
use uuid::Uuid;

#[test]
fn artists_and_albums_group_tracks_by_the_approved_credits() {
    let catalog = Catalog {
        roots: Vec::new(),
        entries: vec![
            entry(
                "ana-shared.wav",
                &["Ana", "Guest"],
                &["Various"],
                Some("Shared"),
                true,
            ),
            entry(
                "ben-shared.wav",
                &["Ben"],
                &["Various"],
                Some("Shared"),
                true,
            ),
            entry("cara-shared.wav", &["Cara"], &[], Some("Shared"), true),
            entry("ana-no-album.wav", &["Ana"], &[], None, true),
            entry("no-artist.wav", &[], &[], None, false),
        ],
    };

    let artists =
        browse_catalog(&catalog, BrowseSelection::View(BrowseView::Artists), 0, 32).unwrap();
    let artist_counts = artists
        .items
        .iter()
        .map(|item| match item {
            BrowseItem::Artist { name, track_count } => (name.clone(), *track_count),
            other => panic!("unexpected Artist browse item: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        artist_counts,
        vec![
            (Some("Ana".to_owned()), 2),
            (Some("Ben".to_owned()), 1),
            (Some("Cara".to_owned()), 1),
            (None, 1),
        ]
    );

    let albums =
        browse_catalog(&catalog, BrowseSelection::View(BrowseView::Albums), 0, 32).unwrap();
    let album_counts = albums
        .items
        .iter()
        .map(|item| match item {
            BrowseItem::Album { key, track_count } => (key.clone(), *track_count),
            other => panic!("unexpected Album browse item: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(album_counts.len(), 3);
    assert!(album_counts.contains(&(
        AlbumKey::Release {
            artist_credits: vec!["Various".to_owned()],
            release_title: "Shared".to_owned(),
        },
        2,
    )));
    assert!(album_counts.contains(&(
        AlbumKey::Release {
            artist_credits: vec!["Cara".to_owned()],
            release_title: "Shared".to_owned(),
        },
        1,
    )));
    assert!(album_counts.contains(&(AlbumKey::NoAlbum, 2)));

    let various_artists_album = browse_catalog(
        &catalog,
        BrowseSelection::Album(AlbumKey::Release {
            artist_credits: vec!["Various".to_owned()],
            release_title: "Shared".to_owned(),
        }),
        0,
        32,
    )
    .unwrap();
    let tracks = various_artists_album
        .items
        .iter()
        .map(|item| match item {
            BrowseItem::Track(entry) => entry.path.clone(),
            other => panic!("unexpected Album track item: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tracks,
        vec![
            PathBuf::from("ana-shared.wav"),
            PathBuf::from("ben-shared.wav")
        ]
    );

    let ana_tracks = browse_catalog(
        &catalog,
        BrowseSelection::Artist(Some("Ana".to_owned())),
        0,
        32,
    )
    .unwrap();
    let tracks = ana_tracks
        .items
        .iter()
        .map(|item| match item {
            BrowseItem::Track(entry) => entry.path.clone(),
            other => panic!("unexpected Artist track item: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tracks,
        vec![
            PathBuf::from("ana-no-album.wav"),
            PathBuf::from("ana-shared.wav")
        ]
    );

    let songs = browse_catalog(&catalog, BrowseSelection::View(BrowseView::Songs), 0, 32).unwrap();
    assert_eq!(songs.items.len(), 5);
    assert!(
        songs
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::Track(entry) if !entry.active))
    );
    let first_song_page =
        browse_catalog(&catalog, BrowseSelection::View(BrowseView::Songs), 0, 2).unwrap();
    assert_eq!(first_song_page.items.len(), 2);
    assert_eq!(first_song_page.next_offset, Some(2));
    let second_song_page = browse_catalog(
        &catalog,
        BrowseSelection::View(BrowseView::Songs),
        first_song_page.next_offset.unwrap(),
        2,
    )
    .unwrap();
    assert_eq!(second_song_page.items.len(), 2);
    assert_eq!(second_song_page.next_offset, Some(4));

    let no_album_tracks =
        browse_catalog(&catalog, BrowseSelection::Album(AlbumKey::NoAlbum), 0, 32).unwrap();
    assert_eq!(no_album_tracks.items.len(), 2);
    assert!(
        no_album_tracks
            .items
            .iter()
            .any(|item| matches!(item, BrowseItem::Track(entry) if !entry.active))
    );
}

fn entry(
    path: &str,
    artists: &[&str],
    album_artists: &[&str],
    release_title: Option<&str>,
    active: bool,
) -> LibraryEntry {
    LibraryEntry {
        id: Uuid::new_v4(),
        path: PathBuf::from(path),
        active,
        metadata: TrackMetadata {
            artists: artists.iter().map(|value| tag(value)).collect(),
            album_artists: album_artists.iter().map(|value| tag(value)).collect(),
            release_title: release_title.map(tag),
            ..TrackMetadata::default()
        },
        duration: Some(Duration::from_secs(100)),
        play_count: 0,
        content_fingerprint: ContentFingerprint {
            version: 1,
            sha256: [0; 32],
        },
    }
}

fn tag(value: &str) -> TaggedValue {
    TaggedValue {
        value: value.to_owned(),
        source: "test".to_owned(),
        field: "test".to_owned(),
    }
}
