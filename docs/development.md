# Kulkán developer guide

## Project map

- `src/main.rs`: GTK application lifecycle, folder chooser, library controls, catalog presentation, and playback transport controls.
- `crates/kulkan-runtime`: managed server process, single-client activation, GStreamer playback, and the public client/server operations.
- `crates/kulkan-ipc`: private Unix socket, peer validation, and the versioned length-delimited message protocol.
- `crates/kulkan-library`: filesystem traversal, read-only metadata discovery through `lofty`, GStreamer duration derivation, scan reconciliation, and catalog presentation data.
- `SPEC.md`: approved product behavior. Treat it as the acceptance contract; resolved Codeberg issues remain the decision history.
- `GLOSSARY.md`: domain language. Read it before changing tests or user-facing terminology.
- `docs/adr/`: architectural decision records, when present.

The server restores and persists selected roots, the scanned catalog/history/preferences, and Queue order/settings in app-owned state, and performs scans. The GTK client asks it to add/remove roots, trigger scans, read summaries, search, and page through catalog entries. Adding a root is not a scan. GTK uses a background worker for IPC and scanning so a long scan does not block the UI thread.

## Build and test

For a normal Rust development environment with GTK 4 development files installed:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --release --locked
```

The scan tests use temporary directories and generated minimal WAV samples. The metadata matrix includes small synthetic files for MP3, WAV/PCM, FLAC, Ogg Vorbis, and Opus. They contain no copyrighted recordings.

## TDD seams

Tests exercise behavior at public boundaries:

- `kulkan_library::scan_library` accepts prior catalog state and selected roots, then returns catalog changes, incomplete paths, and media errors. Integration tests cover duplicate/overlapping roots, selected and nested symlinks, metadata/provenance, read-only media, failed scans, missing files, and removed roots.
- `kulkan_library::search_catalog` applies the public AND criteria and default result ordering. Tests cover case/diacritic-insensitive matching, explicit missing values, numeric ranges, play counts, and ordering.
- `ServerProcess` exposes root management, explicit scanning, state summaries, bounded catalog pages, and search pages. Child-server integration tests verify persistence of roots/catalog/history/preferences and app-state export/import without copying media.
- `ServerProcess`/`ServerClient` expose playback and transport snapshots over the child-server IPC. Integration tests use the real GStreamer pipeline with `fakesink` to cover controls, decoding, and media errors without depending on host audio hardware.
- `ServerProcess`/`ServerClient` expose queue operations and snapshots over the child-server IPC. Integration tests use the real child server and `fakesink` to cover queue ordering, repeats, shuffle, current-entry changes, and failure skipping; GTK queue wiring remains a manual smoke check.
- The same public server seam covers played-threshold preferences, coverage across pauses/seeks/replayed intervals, repeat attempts, and per-entry/all-history clearing. Playback uses a synchronized GStreamer `fakesink` so test timing is repeatable without host audio.
- Unknown media duration is covered through public `scan_library` results; a decodable fixture with unknown duration must produce a positive cached catalog duration.
- `present_catalog_entry` is the GTK presentation-data boundary. It preserves original tag strings and makes inactive entries non-selectable; GTK renders this model.
- Folder-portal grants, removable media, host audio routing, GTK queue/transport/search wiring, selection toggling, timeline seeking, keyboard shortcuts, assistive technology, and distro compatibility require manual checks on real desktops; do not claim they passed based only on unit tests.

For each behavior change, add a failing test at the relevant public seam, implement the smallest vertical slice that makes it pass, and then review adjacent failure paths. Keep filesystem and portal errors visible; do not turn an incomplete scan into confirmed missing files.

### Library content identity

The scanner stores a versioned SHA-256 fingerprint beside each library entry. Fingerprints are format-aware: PCM WAVE hashes its format chunk and sample data; FLAC hashes stream configuration and audio frames; MP3 hashes MPEG audio frames; Ogg Vorbis hashes its identification/setup headers and audio packets; Ogg Opus hashes its decoder configuration (including output gain) and audio packets. Container/tag metadata is excluded. A re-encode is intentionally a different identity. If the canonical representation changes, increment the fingerprint version and add public `scan_library` regression coverage before changing reconciliation behavior.

Identity reconciliation is conservative: same-path exact matches preserve the entry, a unique exact match can transfer an inactive entry's identity on a move, and ambiguous candidates do not transfer history. Files at different paths are never merged.

## Manual library acceptance checklist

1. Build from source on a clean Linux system, then launch Kulkán.
2. Confirm the selection appears, and that the first scan happens only after pressing **Scan Library**.
3. Include MP3, PCM WAV, FLAC, Ogg Vorbis, and Opus samples; verify original visible tags and file locations.
4. Select overlapping roots and a directory symlink as a root; verify one catalog entry per resolved path. Verify a nested symlink is skipped.
5. Disconnect a selected removable volume or revoke access during a test scan. Confirm the scan reports an incomplete path and does not mark last-known entries missing.
6. Reconnect/restore access, rescan, then remove a root and rescan. Confirm entries from a removed root are inactive and cannot be selected.
7. Compare media file checksums before and after scanning to confirm Kulkán did not modify them.
8. Relaunch and confirm selected roots and cached tracks are restored without an implicit scan; then rescan and confirm the desktop folder chooser still permits access to the selected roots.
9. Confirm the scanned Library is visible by default, is ordered by artist/release/track/title, and can be traversed with **Load More Tracks**. Search for a matching subset, then choose **Clear** and confirm the full Library returns.
10. Confirm inactive entries remain visible but cannot be selected or queued. With an empty catalog, confirm the Library explains how to add a folder and scan it.
11. Confirm a plain click selects one track, a second plain click deselects it, Ctrl-click toggles tracks without clearing other selections, and Shift-click selects the inclusive range from the last anchor.

Record the exact Linux distribution, kernel, desktop session, audio sink, and commit tested; do not infer compatibility with other distributions from one successful build.
