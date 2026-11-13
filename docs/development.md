# Kulkán developer guide

## Project map

- `src/main.rs`: GTK application lifecycle, folder chooser, library controls, and catalog presentation.
- `crates/kulkan-runtime`: managed server process, single-client activation, and the public client/server library operations.
- `crates/kulkan-ipc`: private Unix socket, peer validation, and the versioned length-delimited message protocol.
- `crates/kulkan-library`: filesystem traversal, read-only metadata discovery through `lofty`, scan reconciliation, and catalog presentation data. GStreamer-based duration derivation and playback remain later work.
- `SPEC.md`: approved product behavior. Treat it as the acceptance contract; resolved Codeberg issues remain the decision history.
- `GLOSSARY.md`: domain language. Read it before changing tests or user-facing terminology.
- `docs/adr/`: architectural decision records, when present.

The server owns the in-memory library state and performs scans. The GTK client asks it to add/remove roots, trigger scans, read summaries, and page through catalog entries. Adding a root is not a scan. GTK uses a background worker for IPC and scanning so a long scan does not block the UI thread.

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
- `ServerProcess` exposes root management, explicit scanning, state summaries, and bounded catalog pages. Integration tests start the actual child server and verify the client-visible result.
- `present_catalog_entry` is the GTK presentation-data boundary. It preserves original tag strings and makes inactive entries non-selectable; GTK renders this model.
- Folder-portal grants, removable media, host audio routing, assistive technology, and distro compatibility require manual checks on real desktops; do not claim they passed based only on unit tests.

For each behavior change, add a failing test at the relevant public seam, implement the smallest vertical slice that makes it pass, and then review adjacent failure paths. Keep filesystem and portal errors visible; do not turn an incomplete scan into confirmed missing files.

## Flatpak build

The release target is x86-64 on GNOME Platform 51. The sandbox intentionally has no network or broad home-directory access. Media folder access comes from the GTK portal and audio output uses the host audio socket.

```sh
flatpak-builder --user --install --force-clean \
  --repo=flatpak/repo flatpak/build flatpak/org.kulkan.Kulkan.yml
flatpak run org.kulkan.Kulkan
```

The manifest uses the locked workspace and `flatpak/cargo-sources.json` so compilation can run offline inside the sandbox. When `Cargo.lock` changes, regenerate the source list with `flatpak-cargo-generator.py` from the upstream `flatpak-builder-tools` project:

```sh
python3 flatpak-cargo-generator.py Cargo.lock --output flatpak/cargo-sources.json
```

Regenerate, inspect the diff, and verify the Flatpak build before committing dependency changes. Do not hand-edit crate checksums.

## Manual library acceptance checklist

1. Launch the Flatpak under a desktop with `xdg-desktop-portal` and select a folder using **Add Folder…**.
2. Confirm the selection appears, and that the first scan happens only after pressing **Scan Library**.
3. Include MP3, PCM WAV, FLAC, Ogg Vorbis, and Opus samples; verify original visible tags and file locations.
4. Select overlapping roots and a directory symlink as a root; verify one catalog entry per resolved path. Verify a nested symlink is skipped.
5. Disconnect a selected removable volume or revoke access during a test scan. Confirm the scan reports an incomplete path and does not mark last-known entries missing.
6. Reconnect/restore access, rescan, then remove a root and rescan. Confirm entries from a removed root are inactive and cannot be selected.
7. Compare media file checksums before and after scanning to confirm Kulkán did not modify them.
8. Relaunch and rescan to verify portal access behavior on the target desktop. Persistent app-state restoration is a separate feature and must not be inferred from portal permission alone.

The required seven-distro release matrix is listed in [`SPEC.md`](../SPEC.md) and tracked in Codeberg issue #28. Record which systems were actually tested; do not infer compatibility from a successful build on one distribution.
