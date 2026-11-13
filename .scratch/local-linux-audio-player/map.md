# Local Linux Audio Player Spec — Wayfinder Map

> Canonical map: [Local Linux Audio Player Spec — Wayfinder Map](https://codeberg.org/padi/izmusica/issues/1). This file is a local snapshot; Codeberg Issues are canonical.

## Destination

An implementation-ready spec for a single-user, local-only audio player on Linux, built from a separate native desktop client and server process. The spec, not the implementation, is the deliverable.

## Notes

- The user is the sole local listener with a personal collection of local audio files.
- The desktop client and server are separate processes and communicate locally. The client starts the server as needed and stops it when the client exits.
- The app must not be an Electron, Java, or web-based app disguised as native; it should run directly on Linux.
- Required distro families: Ubuntu, Arch, Mint, Debian, Fedora, and openSUSE. x86-64 is the initial CPU architecture.
- The core experience must work offline, without accounts, cloud services, or telemetry.
- The app indexes audio in user-selected folders, only reads the user's media files, and lets the user select matching tracks for a queue. Criteria combine with AND in v1.
- Scanning is explicit/manual; incomplete scans preserve last-known entries, while missing files and removed roots become inactive records with play history retained until explicitly cleared.
- Initial criteria include not-yet-played, artist, album, genre, album/single release year, duration, file location, and player-local play count. Playback is sequential by default, with user-controlled shuffle and repeat.
- The app-wide played threshold is configurable from 1–100% in integer steps, defaulting to 75%; aggregate play counts stay local.
- Consult `AGENTS.md`, `GLOSSARY.md`, and the domain guidance in `docs/agents/domain.md`. No existing code or ADRs impose additional constraints.

## Decisions so far

<!-- One line per resolved ticket, with a link to its detail. -->

- [Find a Linux-native implementation approach that meets the platform requirements](https://codeberg.org/padi/izmusica/issues/2): research recommends Rust, GTK 4, Unix-domain IPC, GStreamer, and Flatpak; alternatives and distro test baselines are documented in the repository research note.
- [Define criteria-based song selection semantics](https://codeberg.org/padi/izmusica/issues/3): criteria use AND, normalized text and range filters, explicit missing-value handling, artist/album/track ordering, and append selected or all matches to the queue.
- [Map audio metadata fields for artist, album, release date, and track number](https://codeberg.org/padi/izmusica/issues/6): research recommends ordered artist credits, format-specific release/track fields, and explicit handling of missing or conflicting values; release dates and single identity have format limits.
- [Define library scanning and media-file identity behavior](https://codeberg.org/padi/izmusica/issues/5): scans are manual, incomplete areas are preserved, and file/history identity follows explicit path, content, duplicate, and symlink rules.
- [Research audio-content fingerprints for move and rename detection](https://codeberg.org/padi/izmusica/issues/7): use a format-aware exact encoded-content digest that excludes tags/container bookkeeping; re-encodes remain new identities, while perceptual matches do not transfer history.
- [Define play-history and configurable played threshold behavior](https://codeberg.org/padi/izmusica/issues/4): count distinct content heard once per attempt, increment on threshold crossing, and persist only local aggregate counts per library entry.

## Not yet specified

- Desktop interaction details such as keyboard controls, accessibility, notifications, and error/recovery behavior.
- Local database/configuration storage, backup/migration expectations, and server failure/reconnection behavior.
- Acceptance criteria and what constitutes a complete first release.

## Out of scope

- Video, streaming services, remote clients, and multi-user access.
- Accounts, cloud services, telemetry, and online-dependent core behavior.
- Editing, moving, renaming, or organizing the user's audio files.
- Automatic DJ/autoplay behavior outside the user-managed queue.
- Advanced OR/grouped criteria and saved rule sets in the first version.
