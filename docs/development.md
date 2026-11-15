# Kulkán developer guide

## Project map

- `src/main.rs`: GTK application lifecycle, folder chooser, library controls, catalog presentation, and playback transport controls.
- `src/library_workspace.rs`: GTK-independent center-destination workflow for Browse and Search, including request effects and stale-completion policy.
- `scripts/gtk-smoke-test.sh`: isolated, headless GTK smoke test for Search/Browse navigation and criteria retention.
- `crates/kulkan-runtime`: managed server process, single-client activation, GStreamer playback, and the public client/server operations.
- `crates/kulkan-ipc`: private Unix socket, peer validation, and the versioned length-delimited message protocol.
- `crates/kulkan-library`: filesystem traversal, read-only metadata discovery through `lofty`, GStreamer duration derivation, scan reconciliation, and catalog presentation data.
- `SPEC.md`: approved product behavior. Treat it as the acceptance contract; resolved GitHub issues remain the decision history.
- `GLOSSARY.md`: domain language. Read it before changing tests or user-facing terminology.
- `docs/adr/`: architectural decision records, when present.

The server restores and persists configured Library folders, the scanned catalog/history/preferences, the startup browse view, and Queue order/settings in app-owned state, and performs scans. The GTK client asks it to add/remove folders, trigger scans, read summaries, browse Artists/Albums/Songs, search, and page through catalog entries. Adding a folder is not a scan. GTK uses a background worker for IPC and scanning so a long scan does not block the UI thread.

## Build and test

For a normal Rust development environment with GTK 4 development files installed:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo build --workspace --release --locked
```

A tracked pre-commit hook is available in `.githooks/pre-commit`. Enable it with:

```sh
git config core.hooksPath .githooks
```

## GTK smoke test

Run the real client in an isolated XDG profile and a headless Weston/Xwayland session:

```sh
./scripts/gtk-smoke-test.sh
```

The smoke test exercises Search submission, Browse navigation, restoring cached Search results, editing criteria, and Clear. It uses an empty Library and does not read or scan the listener's Library. It requires `weston`, `dbus-run-session`, `xdotool`, ImageMagick's `import`, and `tesseract`.

The scan tests use temporary directories and generated minimal WAV samples. The metadata matrix includes small synthetic files for MP3, WAV/PCM, FLAC, Ogg Vorbis, and Opus. They contain no copyrighted recordings.

## TDD seams

Tests exercise behavior at public boundaries:

- `kulkan_library::scan_library` accepts prior catalog state and selected folders, then returns catalog changes, incomplete paths, and media errors. Integration tests cover duplicate/overlapping folders, selected and nested symlinks, metadata/provenance, read-only media, failed scans, missing files, and removed folders. Selected-folder scans must leave unselected configured folders and their entries/history unchanged.
- `kulkan_library::search_catalog` applies the public AND criteria and default result ordering. Tests cover case/diacritic-insensitive matching, explicit missing values, numeric ranges, play counts, and ordering.
- `LibraryWorkspace::dispatch` and `LibraryWorkspace::view` define the deterministic desktop workflow seam. Tests cover Browse navigation, Search criteria/results, paging, hidden Search completion, history refresh, stale requests, and failure behavior without GTK or IPC.
- `ServerProcess` exposes Library-folder management, explicit selected-folder/all-folder scans, state summaries, bounded catalog pages, and search pages. Child-server integration tests verify persistence of folders/catalog/history/preferences and app-state export/import without copying media.
- `ServerProcess`/`ServerClient` expose playback and transport snapshots over the child-server IPC. Integration tests use the real GStreamer pipeline with `fakesink` to cover controls, decoding, and media errors without depending on host audio hardware.
- `ServerProcess`/`ServerClient` expose queue operations and snapshots over the child-server IPC. Integration tests use the real child server and `fakesink` to cover queue ordering, repeats, shuffle, current-entry changes, and failure skipping; GTK queue wiring remains a manual smoke check.
- The same public server seam covers played-threshold preferences, coverage across pauses/seeks/replayed intervals, repeat attempts, and per-entry/all-history clearing. Playback uses a synchronized GStreamer `fakesink` so test timing is repeatable without host audio.
- Unknown media duration is covered through public `scan_library` results; a decodable fixture with unknown duration must produce a positive cached catalog duration.
- `present_catalog_entry` is the GTK presentation-data boundary. It preserves original tag strings and makes inactive entries non-selectable; GTK renders this model.
- The headless smoke test covers Search submission, Browse navigation, Search restoration/editing, and Clear. Folder-portal grants, removable media, host audio routing, GTK queue/transport, selection toggling, timeline seeking, keyboard shortcuts, assistive technology, and distro compatibility still require manual checks on real desktops; do not claim they passed based only on unit tests.

For each behavior change, add a failing test at the relevant public seam, implement the smallest vertical slice that makes it pass, and then review adjacent failure paths. Keep filesystem and portal errors visible; do not turn an incomplete scan into confirmed missing files.

### Library content identity

The scanner stores a versioned SHA-256 fingerprint beside each library entry. Fingerprints are format-aware: PCM WAVE hashes its format chunk and sample data; FLAC hashes stream configuration and audio frames; MP3 hashes MPEG audio frames; Ogg Vorbis hashes its identification/setup headers and audio packets; Ogg Opus hashes its decoder configuration (including output gain) and audio packets. Container/tag metadata is excluded. A re-encode is intentionally a different identity. If the canonical representation changes, increment the fingerprint version and add public `scan_library` regression coverage before changing reconciliation behavior.

Identity reconciliation is conservative: same-path exact matches preserve the entry, a unique exact match can transfer an inactive entry's identity on a move, and ambiguous candidates do not transfer history. Files at different paths are never merged.

## Manual library acceptance checklist

1. Build from source on a clean Linux system, then launch Kulkán. Confirm Songs is the initial startup view and that Artists/Albums/Songs can be selected as the saved startup view in Settings.
2. Open Settings, add multiple Library folders, and confirm the first scan happens only after choosing **Scan selected** or **Scan all**.
3. Include MP3, PCM WAV, FLAC, Ogg Vorbis, and Opus samples; verify original visible tags and file locations.
4. Select overlapping folders and a directory symlink as a Library folder; verify one catalog entry per resolved path. Verify a nested symlink is skipped.
5. Disconnect a selected removable volume or revoke access during a test scan. Confirm the scan reports an incomplete path and does not mark last-known entries missing.
6. Reconnect/restore access and rescan. Remove folders and scan again, including after removing the last configured folder; confirm entries no longer covered by a configured folder become inactive and cannot be selected.
7. Compare media file checksums before and after scanning to confirm Kulkán did not modify them.
8. Relaunch and confirm configured folders, cached tracks, and the saved startup view are restored without an implicit scan; then rescan and confirm the desktop folder chooser still permits access to selected folders.
9. Verify Scan selected affects only selected folders, including when an unselected configured folder is nested under a selected folder; Scan all scans every configured folder. Confirm scans show a completion summary/errors, and closing Settings does not cancel a scan.
10. Browse Artists, Albums, and Songs; verify first-billed artist grouping, Album Artist/release grouping, the No Album placeholder, and the same track-selection/queue actions. Clear a track's history from Songs and Artist/Album tracks and confirm the same browse view remains. Search, navigate away and back, confirm criteria/results persist, and choose Search from results to edit criteria; Clear should leave blank criteria in Search.
11. Confirm inactive entries remain visible but cannot be selected or queued. With an empty catalog, confirm the Library explains how to add a folder and scan it.
12. Confirm a plain click selects one track, a second plain click deselects it, Ctrl-click toggles tracks without clearing other selections, and Shift-click selects the inclusive range from the last anchor.

Record the exact Linux distribution, kernel, desktop session, audio sink, and commit tested; do not infer compatibility with other distributions from one successful build.
