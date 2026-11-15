# Kulkán: application flow and contributor map

> A source-oriented guide to how the current application works, how its major decisions shape the code, and where to make changes. This describes the checked-in implementation; [SPEC.md](../SPEC.md) remains the approved product contract, and the linked Wayfinder issues remain the decision history.

<style>
@page { size: A4 landscape; margin: 14mm; }
body { font: 10pt/1.35 sans-serif; }
h1, h2, h3 { break-after: avoid; }
pre { white-space: pre; font-size: 7.5pt; line-height: 1.2; }
table { width: 100%; border-collapse: collapse; font-size: 8.5pt; }
th, td { padding: 4px 6px; vertical-align: top; }
thead { display: table-header-group; }
tr { break-inside: avoid; }
</style>

## How to read this guide

- **Current behavior** below is grounded in source and tests.
- **Approved behavior** means the consolidated contract in `SPEC.md`, not necessarily a statement that every desktop detail has a dedicated automated test.
- The app is a local, single-user Linux audio player. It reads media but does not edit, move, rename, organize, or upload it.
- Domain terms use [the glossary](../GLOSSARY.md): a **Library entry** is path-backed; a **Track** is the audio item; a **Queue entry** is one occurrence of a Track in the ordered **Queue**; a **Playback attempt** is one pass at hearing a Track.

## One-screen model

```text
 Listener
    │ GTK actions / visible state
    ▼
┌──────────────────────────── Kulkán client ────────────────────────────┐
│ GTK window ── LibraryWorkspace ── background IPC workers              │
│     │              │                          │                        │
│     ├─ Browse/Search state                    │                        │
│     ├─ selection, Settings, Queue rail, player bar                    │
│     └─ activation socket (focus an existing client)                   │
└───────────────────────────────────────────────┼────────────────────────┘
                                                │ private Unix socket
                                                │ versioned length-prefixed JSON
                                                ▼
┌──────────────────────────── kulkan-server ────────────────────────────┐
│ IPC router ── LibraryService ── kulkan-library                        │
│          └── QueueService ── PlaybackService ── GStreamer              │
│                └── PlaybackAttempt                                     │
└───────────────────────────────┬───────────────────────────────────────┘
                                │ reads only
                  ┌─────────────┴─────────────┐
                  ▼                           ▼
           selected folders             local app state
           audio + tags                   library.json
                                           queue.json
```

The client owns presentation and interaction state. The child server owns authoritative Library, Queue, playback, and persistence state. The two processes communicate through the private per-user socket; the app does not use a network service.

## Repository map: where responsibilities live

| Area | Code | Responsibility / change here when… |
|---|---|---|
| GTK shell and orchestration | [`src/main.rs`](../src/main.rs) — `run`, `build_main_window`, `install_library_events`, `execute_workspace_effects`, `perform_playback_request`, `start_catalog_scan`, `install_playback_poll` | Changing window layout, GTK callbacks, Settings, status messages, keyboard actions, worker dispatch, event handling, or wiring the Queue/player to the server. |
| Browse/Search workflow | [`src/library_workspace.rs`](../src/library_workspace.rs) — `LibraryWorkspace::dispatch`, `complete_search`; [`src/search_state.rs`](../src/search_state.rs) — `SearchWorkspace` | Changing destinations, retained criteria/results, paging, async request tickets, stale completion behavior, or workspace state transitions. Keep these GTK-independent. |
| Row selection | [`src/catalog_selection.rs`](../src/catalog_selection.rs) — `SelectionState::select` | Changing plain/Ctrl/Shift click semantics or the range anchor. |
| Local server and public client API | [`crates/kulkan-runtime/src/lib.rs`](../crates/kulkan-runtime/src/lib.rs) — `serve`, `LibraryService`, `ServerProcess`, `ServerClient`, `scan_roots` | Changing process lifecycle, state ownership, persistence coordination, request APIs, or client/server operations. |
| Queue and app-state transfer | [`crates/kulkan-runtime/src/queue.rs`](../crates/kulkan-runtime/src/queue.rs) — `QueueService`, `observe_snapshot`, `advance`, `start_entry` | Changing Queue entries/order, repeat/shuffle, advancement/failures, history coordination, queue persistence, export/import/reset. |
| Audio pipeline | [`crates/kulkan-runtime/src/playback.rs`](../crates/kulkan-runtime/src/playback.rs) — `PlaybackService::play_with_state` | Changing GStreamer pipeline creation, transport, duration/position snapshots, or media errors. |
| Play qualification | [`crates/kulkan-runtime/src/playback_attempt.rs`](../crates/kulkan-runtime/src/playback_attempt.rs) — `PlaybackAttempt::should_qualify` | Changing heard-content coverage or the rules for a qualifying play. See [ADR-0001](adr/0001-playback-attempt-qualification.md). |
| Files, metadata, identity | [`crates/kulkan-library/src/lib.rs`](../crates/kulkan-library/src/lib.rs) — `scan_library_selected`, `read_metadata`, `content_fingerprint`, `derive_duration` | Changing folder traversal, metadata parsing, duration derivation, fingerprints, scan reconciliation, or UI-ready track data. |
| Catalog browsing and criteria | [`crates/kulkan-library/src/browse.rs`](../crates/kulkan-library/src/browse.rs) — `browse_catalog`; [`crates/kulkan-library/src/search.rs`](../crates/kulkan-library/src/search.rs) — `search_catalog` | Changing Artist/Album/track grouping, sort order, criteria matching, or normalized search keys. |
| Wire protocol / socket security | [`crates/kulkan-ipc/src/lib.rs`](../crates/kulkan-ipc/src/lib.rs) — `Message`, `bind_private_socket`, `write_message`, `read_message` | Changing message types, framing/version, peer validation, or private socket creation. |
| Executables | [`crates/kulkan-runtime/src/bin/kulkan-server.rs`](../crates/kulkan-runtime/src/bin/kulkan-server.rs), [`src/main.rs`](../src/main.rs) | Changing server CLI startup or client startup/shutdown. |

`kulkan-library` is independent of the GTK UI. `kulkan-runtime` composes library, IPC, playback, and Queue behavior. The binary crate `kulkan` is the desktop client. Keep behavior at those public seams so it can be tested without GTK or host audio where possible.

## Process startup, ownership, and shutdown

```text
Launch kulkan
    │
    ├─ client.sock answers Activate? ── yes ──► focus existing window; exit
    │
    └─ claim private client.sock
          │
          ├─ claim fails during concurrent launch? ──► retry activation; exit if handled
          │
          └─ create GTK window and ServerState
                │
                ├─ spawn sibling kulkan-server with server.sock + library.json
                │     └─ wait for Ping/Pong and verify peer PID
                │
                ├─ success ──► enable server-backed controls; restore Library + Queue
                │
                └─ failure ──► keep window open; show Server unavailable; Retry / Quit

GTK shutdown ──► request server Shutdown ──► wait for child ──► remove socket
```

The client activation socket prevents a second window/server pair. The server socket is private and independently validates the peer UID; the managed client additionally checks the server PID. Socket setup only removes an owned stale socket after checking that it has no live listener. Socket framing and authentication are implemented in `kulkan-ipc`.

The GTK main thread must stay responsive. Potentially slow IPC (including scans) runs in worker threads; workers return `UiEvent`s to the main loop, which is polled and rendered by `install_library_events`. GTK widgets are not manipulated from workers.

### Crash and retry path

```text
Server request / playback poll fails
       │
       ├─ disable server-dependent actions; preserve visible/cached state
       ├─ first observed playback-server failure ──► one automatic restart
       │       ├─ success ──► reload Library and Queue; current Track starts anew
       │       └─ failure ──► show Retry / Quit; no restart loop
       └─ manual Retry ──► one replacement-server attempt
```

Normal launch and manual Retry restore the current queued Track from the beginning, paused. Automatic recovery restores it from the beginning as a new playing Playback attempt. Already-qualified counts are durable; incomplete attempt coverage is not restored. The GUI retains its window when initial server startup fails.

## Library lifecycle: choose, scan, reconcile, present

```text
Settings: Add Folder ──► persist configured folder only (no scan)
                                   │
                  ┌────────────────┴────────────────┐
                  ▼                                 ▼
          Scan selected folders                Scan all folders
                  └────────────────┬────────────────┘
                                   ▼
       enumerate safely → read tags → fingerprint audio → derive duration
                                   │
                                   ▼
       reconcile against previous Catalog and configured roots
          ├─ same path + same exact content: retain entry/history
          ├─ tag-only edit: refresh tags; retain identity/history
          ├─ same path + changed audio: new identity; old entry inactive
          ├─ unique exact match at moved path: transfer inactive identity/history
          ├─ ambiguous match/re-encode: new identity (no history transfer)
          ├─ complete enumeration confirms missing: inactive, history retained
          └─ incomplete area/media parse error: preserve last-known record
                                   │
                                   ▼
           persist catalog → report summary/errors → refresh current view
```

Important consequences of the scanning decisions:

- Folder selection is explicit; scans are manual. Adding a folder never scans it.
- A selected scan changes only selected configured folders. An unselected folder and its entries/history are not treated as removed, even when roots nest.
- Overlapping roots and an explicitly selected directory symlink deduplicate to one resolved path. Nested symlinks are skipped. Distinct file paths remain distinct Library entries even when their audio is identical.
- An inaccessible root/subtree is **incomplete**, not evidence that its entries disappeared. Only successfully enumerated areas can confirm absence.
- A removed configured folder makes entries no longer covered by any configured folder inactive at reconciliation. Inactive entries remain visible but cannot be queued.
- Media identity is a versioned, format-aware exact SHA-256 fingerprint over audio content and decoder-relevant configuration, excluding tags/container bookkeeping. A re-encode is not an identity match. See the implementation notes in [`docs/development.md`](development.md#library-content-identity).
- Metadata shown to the listener is preserved as read. Search normalization is separate; missing/conflicting tags are not silently fabricated.
- Duration is taken from metadata/discovery where available; otherwise GStreamer processes the audio to derive it. Unknown duration is not treated as zero. A file too malformed to process reports an error.

`scan_library_selected` is the library boundary. The runtime validates the selected paths against configured folders, applies the scan result, persists it, and returns bounded diagnostics and active/inactive totals. Catalog pages are bounded to 32 rows for IPC/UI browsing.

## Browse and Search: user action to visible page

```text
Artists / Albums / Songs ──► WorkspaceEvent::OpenBrowse
                                      │
                                      ▼
                              LoadBrowse effect
                                      │ background IPC
                                      ▼
                       browse_catalog(selection, offset, 32)
                                      │
                                      ▼
                  BrowsePage → ticket check → render current view

Search form ──► parse SearchCriteria ──► SubmitSearch
                                             │
                                             ▼
                                     Search effect (page 0)
                                             │ background IPC
                                             ▼
                                  search_catalog + pagination
                                             │
                                             ▼
                     CatalogPage → ticket check → results view
                                             │
                       ┌─────────────────────┴─────────────────────┐
                       ▼                                           ▼
             Load More (next page)                   Add Selected / Add All Matches
```

`LibraryWorkspace` is the deterministic interaction model: it creates request effects, keeps Search criteria/results while the listener browses elsewhere, and accepts only the currently pending ticket. A late Search completion can be cached without replacing the current Browse destination; an obsolete page cannot replace a newer query. `search_state.rs` owns the criteria/results substate. `main.rs` executes effects on workers and renders the resulting view.

Browse semantics:

- Artists groups each Track once under its first-billed artist; the row keeps full ordered artist credits.
- Albums are grouped by release title and album-artist credits, or first-billed track artist if there is no album-artist credit. **No Album** is a placeholder, not an Album.
- Songs is the full scanned catalog. Artist/Album selection opens that grouping's track list.
- Inactive entries may be browsed as unavailable but cannot be selected for playback/queue actions.
- Track order is first artist, release/date, track number, title (missing sort values last; stable path tie-breaks may follow).
- The persistent Queue rail and player bar remain beside the center Browse/Search workspace.

Search semantics:

- Criteria are AND-combined. Text matching is case- and diacritic-insensitive substring matching; displayed metadata remains original.
- Criteria include text, not-yet-played, artist, album, genre, release year, duration, file location, and play count. Missing values match only explicit missing criteria where supported; duration cannot be filtered as missing.
- Numeric criteria accept exact values or inclusive ranges. Invalid ranges are rejected.
- Results are paged; “Add All Matches” walks every matching page before appending the Tracks in displayed order. Queue duplicates are allowed.
- Returning to Search restores the previous criteria/results. Search from its results returns to criteria editing. Clear resets both and leaves an empty criteria view.

## Queue and playback

```text
Catalog selection / Search results
               │ append paths (one occurrence per selected item)
               ▼
      QueueService ── persists Queue entries, order, repeat, shuffle
               │ choose/play/advance one occurrence
               ▼
      PlaybackService ── GStreamer playbin ── local audio sink
               │ snapshots (track, status, position, duration, error)
               ▼
      PlaybackAttempt ── heard interval coverage ── qualifies once?
               │ yes                                  │ no / stop early
               ▼                                      ▼
LibraryService persists count; emit update      no history increment
               │
               └─ Queue snapshot updates GTK rail, row count, player bar
```

Queue behavior is owned by `QueueService`, not the GTK row model:

- A Queue entry has a unique occurrence ID; the same Track path may occur repeatedly.
- New Tracks append to the pending sequence. Upcoming entries may be reordered/removed. Removing the current entry advances; Clear stops and empties the Queue.
- Repeat is Off, One, or All. Shuffle changes upcoming order without interrupting the current entry; a Repeat All cycle starts fresh. Failed entries remain visible and are skipped for the current cycle, not retried automatically later.
- Previous goes to the previous Queue entry if the current Track has played less than two seconds; otherwise it restarts the current Track.
- The client polls snapshots while active, applies count updates to visible rows, and refreshes Browse/Search when history changes.
- GStreamer must have the required decoding and audio-sink plugins available locally. Kulkán does not download codecs. AAC/M4A is unsupported; media errors are surfaced in-app.

### Qualifying-play rule and ADR alignment

One Playback attempt begins when a Track starts and ends on stop, selecting another Track, app exit, or a repeat pass. Pausing preserves coverage. Seeking over unheard material adds no coverage; replayed intervals count only once. The threshold (default 75%, configurable 1–100%) is captured at attempt start. On reaching it, the play count increments immediately, at most once, and remains counted even if playback later fails. A new repeat pass is a new attempt. With unknown duration, natural completion qualifies; seeking to the end alone does not.

This matches [ADR-0001: isolate Playback attempt qualification](adr/0001-playback-attempt-qualification.md): `PlaybackAttempt` owns interval/threshold interpretation in a private runtime module; `QueueService` owns attempt lifecycle and persists the resulting count through Library state. This is intentionally not folded into scanner or GTK state.

## State: what survives and where

| State | Owner / on-disk location | Restore behavior |
|---|---|---|
| Configured Library folders, catalog cache, metadata, content fingerprint, play counts, threshold, startup Browse view, applied CSS | `LibraryService`, `$XDG_DATA_HOME/org.kulkan.Kulkan/library.json` (or `$HOME/.local/share/...`) | Restored without an implicit scan. Cache is rebuildable through an explicit scan; CSS also travels with exported app state. |
| Queue entries/order, current occurrence, repeat, shuffle | `QueueService`, sibling `queue.json` | Current Track starts from the beginning, paused on normal startup/manual Retry. |
| In-flight Playback attempt coverage | In-memory `QueueService` / `PlaybackAttempt` | Not persisted; recovery starts a new attempt. |
| Client/server sockets | `$XDG_RUNTIME_DIR/org.kulkan.Kulkan/{client,server}.sock` | Runtime-only; private directory/socket permissions and peer checks apply. |
| Exported backup | Listener-selected local JSON file | Includes app-owned state only; does not include media. |

State writes use temporary files, restrictive file permissions, sync, and rename for the Library and Queue stores. Export/import/reset are coordinated by the Queue service with rollback on a failed replacement. Unknown play count is treated as zero. History is retained for inactive entries until explicitly cleared.

## Decisions and implementation traceability

`SPEC.md` consolidates approved behavior and links to the [Wayfinder decision map (issue #1)](https://github.com/itzam-na/kulkan/issues/1) and its decision issues. This table connects those decisions to their code/test seams; it is not a substitute for the issue history.

| Decision topics | Contract location | Primary implementation / verification seam |
|---|---|---|
| Linux-native stack, platform, formats, release scope (issues #2, #8, #9, #21) | `SPEC.md` §§1–3, 11 | `Cargo.toml`, `src/main.rs`, server binary, `playback.rs`; playback integration tests; real desktop/codec availability remains environment-dependent. |
| AND criteria, duration, genre/metadata mapping, Search UI (issues #3, #6, #14, #16, #17) | `SPEC.md` §§5–6, 9 | `search.rs`, `browse.rs`, `LibraryWorkspace`; library `search`/`catalog_browsing`/`catalog_presentation` tests and workspace unit tests. |
| Scanning, path/content identity, metadata (issues #5, #6, #7) | `SPEC.md` §§4–5 | `scan_library_selected`, `read_metadata`, `content_fingerprint`; library `scanning` tests and generated metadata fixtures. |
| Queue controls and Queue UI (issues #10, #11) | `SPEC.md` §7, §9 | `queue.rs`, GTK `QueueUi`; runtime `library_operations`/`playback` tests; GTK queue wiring also needs desktop smoke/manual coverage. |
| History/threshold and qualifying Playback attempt (issue #4; ADR-0001) | `SPEC.md` §8 | `playback_attempt.rs`, Queue history coordination; focused attempt tests plus runtime playback tests. |
| Persistence, transfer, server lifecycle/recovery (issues #12, #13) | `SPEC.md` §10 | `LibraryService`, `QueueService`, `ServerProcess`, `ClientInstance`; runtime `server_lifecycle` and `client_instance` tests. |
| First-release acceptance (issue #15) | `SPEC.md` §11 | `docs/development.md` manual acceptance checklist, workspace test suite, `make gtk-smoke-test`; desktop portals, host audio, accessibility, and distro checks remain manual. |

**Deliberate v1 boundaries:** no streaming, cloud/network service, multi-user support, playlists, automatic DJ behavior, AAC/M4A, saved rule sets, or grouped OR criteria. Their absence is a product decision, not an unimplemented flow to fill in casually. For resolved behavior that conflicts with the code, update the implementation or explicitly reopen/correct the decision/spec; do not quietly redefine it in a guide.

## Tests and contributor workflow

| Change | Start with | Also check |
|---|---|---|
| Scan, metadata, identity, duration | `kulkan_library::scan_library` integration tests | inaccessible paths preserve entries; selected scans do not mutate other roots; media fixtures remain synthetic. |
| Search/browse | public `search_catalog` / `browse_catalog` tests | AND semantics, missing values, normalized matching, stable order, paging. |
| Workspace behavior | `LibraryWorkspace::dispatch` and `view` tests | stale completions, hidden Search results, paging and refresh behavior without GTK/IPC. |
| Queue/playback/history | runtime server/client integration tests, using GStreamer `fakesink` | repeat/shuffle/failure flows, pauses/seeks/replay, history persistence and retry. |
| IPC/process lifecycle | `kulkan-ipc` and runtime lifecycle tests | protocol version/frame bound, peer validation, single-instance launch, clean shutdown/recovery. |
| GTK wiring | `make gtk-smoke-test` plus manual checklist in `docs/development.md` | queue/transport, portals, keyboard, assistive technology, host audio, removable media. |
| Documentation/source map | `make docs-check` | stable source references and decision links, then the complete locked workspace test suite. |

Useful commands:

```sh
make check          # format + workspace tests + Clippy
make docs-check     # source/decision/document cross-check + workspace tests
make docs           # target/docs/kulkan-app-flow.html and .pdf
make gtk-smoke-test # isolated headless GTK search/browse smoke test
```

### Change guide by feature

1. **Find the owning boundary** from the repository map. Keep UI state, domain rules, and durable state in their existing layers.
2. **Read the contract and vocabulary**: relevant `SPEC.md` section, `GLOSSARY.md`, the matching issue(s), and applicable ADR. The issue map is the authority if the consolidated spec is stale.
3. **Add a failing test at the public seam** before implementation. Add a vertical slice through IPC/GTK only if that flow crosses those boundaries.
4. **Preserve negative paths**: scan incompleteness is not confirmed absence; unavailable entries cannot be played; persistence failure must not silently discard state; stale UI responses must not overwrite newer state.
5. **Update this guide when an architectural flow/boundary changes**, and run `make docs-check`. Update `SPEC.md`/ADR only when the approved decision itself changes, not to disguise a code/spec mismatch.

## Documentation consistency check: what it can and cannot prove

`make docs-check` checks that the guide's cited files and stable source symbols still exist, that its decision issue links are still present in `SPEC.md`, and that the workspace test suite passes. This catches common structural drift (renames, removed seams, missing decision traceability) and behavioral regressions represented by tests. It **cannot** prove that prose is semantically true, that all approved behavior has been implemented, or that manual desktop acceptance passed. Review implementation-status prose and the manual acceptance checklist whenever a behavior changes.
