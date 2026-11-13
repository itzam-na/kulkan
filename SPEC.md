# Local Linux Audio Player Specification

**Status:** Consolidated v1 specification, ready for implementation planning
**Audience:** Product and implementation contributors

This document consolidates the approved Wayfinder decisions for a single-user, personal Linux audio player. The [Codeberg Wayfinder map](https://codeberg.org/padi/izmusica/issues/1) and its linked decision issues are the decision history; this file is the implementation-facing specification. If a detail here conflicts with a resolved issue, correct the document to match the issue rather than silently changing the decision.

## 1. Product goal and boundaries

Build a native Linux desktop audio player for one listener's local music collection. The listener chooses folders, scans them explicitly, searches and filters the catalog, and selects tracks for a user-managed playback queue.

The core experience must work offline. The application is for personal use and must not edit, move, rename, or organize the user's media files. V1 is audio-only and single-user.

### Out of scope for v1

- Video, streaming services, remote clients, multi-user access, accounts, cloud services, telemetry, or online-dependent core behavior.
- Automatic DJ/autoplay behavior outside the listener-managed queue.
- Advanced OR/grouped criteria or saved rule sets.
- AAC/M4A and distro-native packages.
- Public Flathub distribution and selecting an application source-code license.

## 2. Architecture and technology constraints

- Implement the desktop client and local server as separate native processes using Rust.
- Use GTK 4 (`gtk4-rs`) for the desktop UI, a private per-user Unix-domain stream socket for local IPC, and GStreamer 1.x (`gstreamer-rs`) for playback and media discovery.
- Do not use Electron, Java, or a web runtime as the application shell.
- Run one application/server instance per user. The client starts the server as needed and shuts it down when the client exits; supervision follows [Server recovery](#10-local-state-and-server-recovery).
- Use a versioned, length-delimited IPC protocol over the private socket; restrict socket-directory permissions and validate the peer.
- V1 packaging is Flatpak-only for x86-64, using the GNOME runtime/SDK or a compatible GStreamer runtime extension. Use a private/local Flatpak repository; updates use `flatpak update`.
- Keep the client and child server in the same sandbox. Grant no network access and avoid broad host-filesystem permissions. Use GTK folder portals for explicitly selected media roots, retaining only the access needed for rescans, and grant the host audio socket permission required for playback.

## 3. Supported platforms and media

### Linux test baselines

Test the Flatpak on each of these x86-64 environments:

1. Ubuntu 26.04 LTS
2. Linux Mint 22.3
3. Debian 13.7
4. Current Arch rolling
5. Fedora 45
6. openSUSE Tumbleweed
7. openSUSE Leap 16.1 (document its x86-64-v2 CPU requirement)

Other x86-64 systems with a compatible Flatpak runtime are best-effort; compatibility with every distribution/desktop combination is not promised.

### Supported audio formats

V1 supports MP3, WAV/PCM, FLAC, Ogg Vorbis, and Opus. The Flatpak must include the required GStreamer elements in its runtime or compatible extension, including `mpg123audiodec`, `mpegaudioparse`, `wavparse`, `flacdec`, `vorbisdec`, `opusdec`, and an audio sink; do not rely on host-installed plugins. Do not download missing decoders during playback; report an actionable unsupported-format error. AAC/M4A is deferred.

## 4. Library, scanning, and file identity

The listener selects library roots. Scanning is explicit/manual; do not continuously watch roots or modify media files.

### Catalog entries and scans

- A Library entry represents one resolved file path. Files with identical audio at distinct paths remain separate entries.
- The same path reached through overlapping roots, or through an explicitly selected directory symlink and its target, appears once.
- Skip nested symlinks. Follow a directory symlink only when the listener explicitly selected it as a root.
- A scan that cannot read a root or subtree is incomplete: preserve its last-known entries and report the incomplete area. Only successfully enumerated areas may establish that an entry is missing.
- Missing files and removed roots become inactive, non-selectable entries; retain their play history until the listener clears it. A file returning at the same path restores that entry.
- Metadata-only changes refresh tags and preserve identity/history. Changed audio content at the same path creates a new identity with zero plays; retain the previous identity as inactive/recoverable.
- A move or rename transfers history only when exactly one inactive entry matches the audio content while ignoring metadata tags. Ambiguous matches and re-encodes create new identities.
- The catalog has one entry per resolved path; copies at distinct paths are never merged.

### Audio-content identity

Use a format-aware exact encoded-content SHA-256 digest that excludes tags and container bookkeeping, but includes the codec configuration needed to interpret the stream and the ordered encoded audio frames/packets. For PCM WAV, include the format description and sample data. Compute linearly without decoding. A re-encode is a new identity; perceptual/similarity matches must not transfer history. Transfer history only for a unique exact match.

The detailed rationale and format-specific source notes are in the [audio-content fingerprint research](https://codeberg.org/padi/izmusica/src/branch/research/audio-content-fingerprint/.scratch/local-linux-audio-player/research/audio-content-fingerprint.md).

## 5. Metadata and catalog values

Preserve original metadata for display. Normalize separately for search and filtering. Missing or conflicting metadata must be deterministic and must not be silently fabricated.

### Artist, release, and track fields

- Preserve ordered track-artist credits separately from album-artist credits.
- Use ID3 TPE1, Vorbis `ARTIST`, and WAV IART for ordered track artist credits. Split only the defined ID3v2.3 slash separator; do not infer multiple artists from arbitrary punctuation.
- Use TALB/`ALBUM`/IPRD as the release title. The formats do not provide a consistent album-versus-single type; a Single is a one-track album. Tracks without album identity belong to “No Album,” which is a placeholder, not an album.
- Use only semantically valid release-date fields: ID3v2.4 TDRL is the exact release-date field. Do not treat TDRC, Vorbis/FLAC DATE, or WAV ICRD as a release date.
- Use TRCK/`TRACKNUMBER`; WAV ITRK is a compatibility extension. Parse `number/total` where present. Preserve absence and conflicts deterministically.
- Every credited artist is searchable, but list each track once under its first-billed artist and show the full credit.

The complete cited field mapping is documented in the [audio metadata mapping research](https://codeberg.org/padi/izmusica/src/branch/research/audio-metadata-mapping/.scratch/local-linux-audio-player/research/audio-metadata-mapping.md).

### Genre

- MP3: use ID3v2 TCON in preference to the legacy ID3v1 genre byte.
- FLAC, Ogg Vorbis, and Opus: use Vorbis Comment/OpusTags `GENRE` values.
- WAV/PCM: use RIFF INFO IGNR in preference to embedded ID3.
- Preserve multiple genre values as an ordered list and keep source values/provenance distinct from normalized search keys. Honor format-defined value boundaries; do not split arbitrary punctuation such as commas, slashes, semicolons, or whitespace.
- A genre criterion matches if any stored value matches. If there are no recognized non-empty values, genre is missing and matches only an explicit “is missing” criterion.
- For conflicting representations, apply the precedence above and retain losing values as provenance rather than silently unioning contradictory tags.

See the [genre metadata research note](https://codeberg.org/padi/izmusica/src/branch/research/genre-metadata-mapping/.scratch/local-linux-audio-player/research/genre-metadata-mapping.md). Validate ID3v2.3 variants and parsing against the shipped GStreamer runtime.

## 6. Criteria and library search

Criteria combine with AND. V1 criteria are not-yet-played, artist, album, genre, album/single release year, duration, file location, and player-local play count.

- Text criteria use case- and diacritic-insensitive substring matching; display the original metadata.
- Year and duration support exact values and inclusive ranges. Year means the album/single release year.
- Missing artist, album, genre, and year values match only an explicit “is missing” criterion. An unknown play count is zero.
- Duration must be available for criteria filtering. During media discovery, use available container/stream duration; if discovery cannot provide it, derive duration by processing the audio content through GStreamer and cache it in the rebuildable catalog. Do not expose “Duration is missing” as the normal fallback or coerce an unknown value to zero. If the file is unreadable or too malformed to derive duration, report a media/discovery error.
- A not-yet-played Track has zero qualifying plays.
- Default result order is artist A–Z, then albums by release date oldest-to-newest, then tracks by ascending track number. Missing dates/numbers sort last; title breaks ties.
- A listener may append selected results or all results to the end of the queue in displayed order. Duplicate queue entries are allowed.
- Advanced OR/grouped rules and saved rule sets are not part of v1.

## 7. Playback queue and controls

The queue is an ordered sequence of Queue entries; the same Library entry may occur more than once. Playback is sequential by default.

- Repeat modes are Off, One, and All. With Off, stop after the last entry and keep the queue intact. One repeats the current entry until skipped or changed. All starts a new queue cycle.
- Shuffle plays each queued occurrence once per cycle without interrupting the current entry. Repeat All creates a fresh shuffle each cycle. If Shuffle is turned off, retain manual reordering performed while it was on.
- Adding tracks appends them to the pending queue. The listener may reorder or remove upcoming entries.
- Removing the current entry advances playback. Clearing the queue stops playback and empties it.
- A failed entry is marked and skipped for the current cycle; keep it available for manual retry/removal and do not automatically retry it in later cycles. A Repeat One failure disables that repeat and advances. If no playable entries remain, stop and report the error.

## 8. Playback attempts and play history

A Playback attempt is one playback of one Track, paused/resumed as needed. It ends when playback is stopped, another Track is selected, the app closes, or repeat starts a new pass.

- The app-wide played threshold is an integer percentage from 1–100, default 75%.
- A qualifying attempt requires hearing the configured share of distinct Track content. Pauses preserve coverage; seeking over content adds none; replaying heard portions does not count twice.
- Increment the play count immediately when the threshold is reached, at most once per attempt. Skipping before the threshold adds nothing; skipping after it keeps the count. Each repeat pass is a new attempt.
- Changing the threshold affects future attempts only; do not recalculate history or alter an in-progress attempt.
- If playback still has no usable duration after discovery/computation, only natural completion qualifies; seeking to the end does not. Unreadable/malformed files that cannot be processed to derive duration report a media/playback error.
- Persist aggregate counts locally per Library entry, without play timestamps. An unknown count is zero. The listener may clear one entry or all history, including inactive entries.

## 9. Desktop interaction

### Approved layout and search

- Use the Library-first layout: library navigation at left, primary content in the center, and a persistent queue rail at right.
- While idle, show current-track artwork/details in the center. An explicit search replaces that center content with matching results; clearing search restores the artwork.
- Provide Basic search with inline criteria chips and Advanced search with a rule composer. Both preserve the current query/results and the queue rail.
- Boolean true criteria display as their label alone (for example, “Not-yet-played”); false prefixes “not.” Constrain long labels to the chip/card with ellipsis and make the full label available as a tooltip.
- Search ignores diacritics while results preserve original metadata.
- The approved throwaway UI prototypes are discussion artifacts only; do not merge them as production UI.

### Keyboard, accessibility, and feedback

- Core workflows must be operable by keyboard, with a small documented set of common shortcuts. Custom shortcut remapping is not required for v1.
- GTK controls must expose accessible names, roles, and states so assistive technologies can discover and operate the core workflows. No formal WCAG conformance level is claimed.
- Do not send native desktop notifications. Playback, server, and recovery feedback stays in the app.

## 10. Local state and server recovery

### Persistence and transfer

Persist selected library roots, the rebuildable metadata/catalog cache, user preferences (including played threshold), per-entry play counts including inactive entries, and the current queue/order plus shuffle/repeat settings.

- Derive the cache from read-only media files; rebuild it through explicit rescan.
- On restore, start the current queued Track from the beginning as a new Playback attempt; do not restore a partial playback position.
- Provide user-initiated export/import of app-owned state for personal backup/migration. Exclude media files and cloud sync.
- Preserve controls for clearing individual or all play history.

### Client/server lifecycle

- Enforce one application/server instance per user. The first GTK client starts one child server on a private per-user Unix socket. A subsequent launch focuses the existing client rather than starting another server.
- Startup is idempotent. Clean a stale socket only after confirming no live server owns it.
- If initial server startup fails, keep the client open with a clear “Server unavailable” state and Retry/Quit actions. Preserve cached library and queue; server-dependent actions are unavailable.
- If the server crashes during playback, stop transport and make one automatic restart/reconnect attempt. On success, restore app state and queue, restart the current Track from the beginning as a new attempt, keep already-qualified counts, and discard incomplete progress.
- On automatic recovery failure, preserve queue/cache and offer Retry/Quit without a retry loop. Manual Retry may start one replacement server.

## 11. Acceptance criteria for a first release

A first release is complete when:

1. An installable Flatpak works locally; public-store publication is not required.
2. Every settled requirement has a verifiable acceptance check, automated where practical and otherwise documented for manual execution.
3. Installation, launch, and representative playback pass on all seven required platform baselines; verify selected-folder portal access across restarts, removable-media roots, and host audio routing.
4. Every supported audio format passes playback on every required platform baseline. Deeper automated coverage may run on a primary environment.
5. Core library, search/criteria, queue/playback, history, persistence/export/import, recovery, keyboard, and accessibility workflows pass against their specified behavior, including offline operation.

Release blockers include crashes or hangs; modification of user media; loss of app-owned state; inability to complete a core workflow; or failure of any required platform/format combination. Document and triage lesser defects. Do not set numeric performance targets until reference hardware and library size are defined; crashes, hangs, and unusable core workflows remain blockers.

## 12. Implementation details intentionally left open

The spec defines behavior, not a database schema, storage engine, IPC message schema, or exact keyboard shortcut assignments. The IPC framing must be versioned and length-delimited as specified above. The implementation must preserve the externally specified behavior and acceptance checks. Choosing an application source-code license is out of scope for v1.

## 13. Decision traceability

- [Issue #2 — Linux-native implementation research](https://codeberg.org/padi/izmusica/issues/2)
- [Issue #3 — criteria-based selection semantics](https://codeberg.org/padi/izmusica/issues/3)
- [Issue #4 — play-history and played-threshold behavior](https://codeberg.org/padi/izmusica/issues/4)
- [Issue #5 — library scanning and file identity](https://codeberg.org/padi/izmusica/issues/5)
- [Issue #6 — artist, album/release, date, and track metadata](https://codeberg.org/padi/izmusica/issues/6)
- [Issue #7 — audio-content identity research](https://codeberg.org/padi/izmusica/issues/7)
- [Issue #8 — native technology stack](https://codeberg.org/padi/izmusica/issues/8)
- [Issue #9 — platform, packaging, and audio-format matrix](https://codeberg.org/padi/izmusica/issues/9)
- [Issue #10 — playback queue controls](https://codeberg.org/padi/izmusica/issues/10)
- [Issue #11 — queue UI prototype](https://codeberg.org/padi/izmusica/issues/11)
- [Issue #12 — persistence and state transfer](https://codeberg.org/padi/izmusica/issues/12)
- [Issue #13 — client/server recovery](https://codeberg.org/padi/izmusica/issues/13)
- [Issue #14 — library search and criteria UI prototype](https://codeberg.org/padi/izmusica/issues/14)
- [Issue #15 — first-release acceptance criteria](https://codeberg.org/padi/izmusica/issues/15)
- [Issue #16 — duration derivation and filtering](https://codeberg.org/padi/izmusica/issues/16)
- [Issue #17 — genre metadata mapping](https://codeberg.org/padi/izmusica/issues/17)
