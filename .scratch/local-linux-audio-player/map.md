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
- Initial criteria include not-yet-played, artist, album, genre, album/single release year, duration, file location, and player-local play count. Playback is sequential by default, with user-controlled shuffle and repeat.
- A track counts as played after 75% of its duration by default; the threshold is configurable and play history stays local.
- Consult `AGENTS.md`, `GLOSSARY.md`, and the domain guidance in `docs/agents/domain.md`. No existing code or ADRs impose additional constraints.

## Decisions so far

<!-- One line per resolved ticket, with a link to its detail. -->

- [Find a Linux-native implementation approach that meets the platform requirements](https://codeberg.org/padi/izmusica/issues/2): research recommends Rust, GTK 4, Unix-domain IPC, GStreamer, and Flatpak; alternatives and distro test baselines are documented in the repository research note.
- [Define criteria-based song selection semantics](https://codeberg.org/padi/izmusica/issues/3): criteria use AND, normalized text and range filters, explicit missing-value handling, artist/album/track ordering, and append selected or all matches to the queue.

## Not yet specified

- Confirm the researched implementation and packaging recommendation, including exact supported release baselines, update mechanism, and final format support.
- Library scan behavior beyond indexing selected folders and allowing a rescan: file changes, deleted/moved files, duplicate paths, symbolic links, and preservation of play history.
- Metadata mapping across supported formats: source fields and precedence for multi-artist credits, album/single identity and release dates, and track numbers.
- Play-history details beyond the 75% configurable default: threshold range and control, seeking/skipping, repeated plays, and how history is associated with moved or replaced files.
- Queue controls and edge cases, including shuffle/repeat behavior, editing the queue, and what happens when playback ends or a file fails.
- Desktop interaction details such as keyboard controls, accessibility, notifications, and error/recovery behavior.
- Local database/configuration storage, backup/migration expectations, and server failure/reconnection behavior.
- Acceptance criteria and what constitutes a complete first release.

## Out of scope

- Video, streaming services, remote clients, and multi-user access.
- Accounts, cloud services, telemetry, and online-dependent core behavior.
- Editing, moving, renaming, or organizing the user's audio files.
- Automatic DJ/autoplay behavior outside the user-managed queue.
- Advanced OR/grouped criteria and saved rule sets in the first version.
