# Kulkán user guide

This guide describes the library, playback, search, queue, play-history, persistence, and state-transfer workflows currently implemented in the local development build.

## Install and launch

Kulkán is currently distributed as a private/local Flatpak, not through Flathub or distro package repositories. Follow [`flatpak/README.md`](../flatpak/README.md) to build and install it for your user. Launch it with:

```sh
flatpak run org.kulkan.Kulkan
```

The application works offline and the Flatpak has no network permission.

## Add a music folder and scan

1. Choose **Add Folder…**. GTK opens the desktop's folder chooser/portal; select a folder that contains your music.
2. Repeat to add more roots. Overlapping roots are allowed and do not create duplicate catalog entries.
3. Select **Scan Library** when you want Kulkán to inspect the selected folders. Scanning is explicit; Kulkán does not watch them continuously.
4. Read the status line for the number of active and inactive entries, incomplete areas, and media errors.

Kulkán remembers selected roots and the last scanned catalog locally. After relaunch, cached tracks appear without an automatic scan; choose **Scan Library** to refresh them. The portal must grant access to each selected folder; if a removable drive is disconnected or access is revoked, reconnect it or select the folder again before rescanning.

## Browse the catalog

The catalog displays original tag text when available: title, artist credit, release title, release year, track number, genre, duration, and canonical file location. Missing tags remain missing; Kulkán does not guess metadata from filenames or filesystem timestamps. Click a selected row again to deselect it.

## Search the Library

Enter a phrase and choose **Search** to show matching tracks without changing the Queue. Basic search ignores case and diacritics while preserving original displayed tags; the active filters appear as inline chips. **Advanced search** adds AND-combined artist, album, genre, year, duration, file-location, play-count, and Not-yet-played criteria. Use **is missing** for supported metadata fields. Year, duration (seconds), and play count accept an exact number or an inclusive `min..max` range. Results are ordered by artist, release date, track number, then title. Choose **Add Selected** or **Add All Matches** to append results in that order; duplicates are allowed. **Clear** returns to the idle Library view.

Supported discovery formats are MP3, PCM WAV, FLAC, Ogg Vorbis, and Ogg Opus. AAC/M4A is not supported and is reported in the scan status rather than silently ignored. A supported file that is malformed or cannot be read is reported as a media error rather than silently treated as a valid track.

## Play a track

Select one or more active catalog rows and choose **Play/Pause** to start them. The transport controls are ordered **Previous**, **Stop**, **Play/Pause**, **Next**. Play/Pause pauses or resumes active playback; clicking the timeline seeks. Previous goes to the prior queue entry when the current track has played for less than two seconds, and otherwise restarts the current track. The current artist, title, elapsed time, and duration appear below the catalog. Playback uses the GStreamer plugins packaged with Kulkán and works offline; it does not download decoders or depend on host-installed codec plugins.

## Play history

The Library shows **Not-yet-played** until a track qualifies, then shows its local play count. Set the app-wide **Played threshold (%)** from 1–100; it defaults to 75%, and changes apply to new playback attempts. Kulkán counts distinct portions heard during an attempt, so pauses preserve progress, seeks add none, and replayed portions count only once. Use a row's **Clear history** button or **Clear All Play History** to reset counts, including inactive entries.

## Queue

Use **Add Selected** or **Add All Matches** to append entries from search results; outside search, use **Add All Tracks**. Duplicate entries are allowed. The queue rail supports playing an entry, reordering upcoming entries, removing entries, clearing the queue, and changing repeat and shuffle modes. Queue order and repeat/shuffle settings persist across relaunches. A restored current track starts from the beginning.

## Backup and keyboard shortcuts

Use **Export State…** and **Import State…** for a local backup or migration. The backup contains Kulkán-owned roots, cached catalog/history, preferences, and queue state; it does not include or modify media files. **Reset App State…** clears the same app-owned data while leaving audio files untouched.

- **Ctrl+F** focuses Basic search.
- **Ctrl+Enter** adds selected results to the Queue.
- **Ctrl+Space** toggles playback.
- **Ctrl+Left** and **Ctrl+Right** go to Previous and Next.
- **Ctrl+S** scans the Library when a text field is not focused.
- **Ctrl+Shift+E** exports state; **Ctrl+Shift+I** imports state.
- **Escape** clears an active search, or clears Library/Queue selection when no search is active.

When media metadata does not provide a duration, Kulkán uses GStreamer to discover or derive one and stores it in the catalog. If a file cannot be decoded or its duration cannot be established, the status line reports a playback/discovery error. Playback errors include the GStreamer diagnostic where available.

## Rescans and missing files

- Kulkán reads media; it does not edit, move, rename, or organize audio files.
- A completed scan that confirms a file is gone keeps its catalog record but marks it inactive.
- Removing a library root and scanning makes entries known only through that root inactive.
- If a root or subtree cannot be enumerated, Kulkán reports the affected path and preserves last-known entries there. Reconnect the drive or restore access and scan again.
- Nested symbolic links are skipped. A directory symlink can be scanned when it is explicitly selected as a library root.
- Reaching a file through overlapping roots or through a selected directory symlink does not duplicate its canonical path. Separate copies at distinct paths remain separate entries.
- Editing tags refreshes the displayed metadata without changing the entry's identity. Replacing the audio content at the same path creates a new entry and retains the old one as inactive.
- A rename or move preserves identity only when exactly one inactive entry has the same exact encoded audio content. Ambiguous matches and re-encodes start new entries, keeping play history associated with the correct entry.

## Server startup

The GTK client starts a local Rust server. If startup fails, use **Retry** to make one manual replacement attempt or **Quit** to close the application. A second launch activates the running client rather than starting another server.

## Report a problem

Issues are tracked at [Codeberg](https://codeberg.org/padi/izmusica/issues). Include the Kulkán version/build, Linux distribution and desktop session, the affected file format, and the exact steps and status/error text. Do not attach personal audio files unless you have made a safe, non-private test sample.
