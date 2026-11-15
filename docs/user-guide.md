# Kulkán user guide

This guide describes Kulkán's current library, playback, search, queue, play-history, persistence, and state-transfer workflows. Some approved behavior still requires manual desktop acceptance; see the [developer guide](development.md) and [product specification](../SPEC.md).

## Install and launch

On x86-64 Arch Linux, build and install the local source package:

```sh
sudo pacman -S --needed base-devel git
git clone https://github.com/itzam-na/kulkan.git
cd kulkan
make package-install
```

Run `make package-install` as your regular user, not with `sudo`; it builds the package, syncs dependencies as needed, and uses `sudo` for pacman. Use `make package` to build without installing. The package is built locally from source; it is not a prebuilt release or an AUR package. The `PKGBUILD` keeps its work tree and source cache in `.makepkg-build/`, separate from the source checkout. Remove it with `make package-uninstall`.

On other Linux distributions, build and run the player with Make:

```sh
make release
make run-release
```

After installing from the source checkout, launch the installed app with `make launch` or from the desktop application menu.

The application works offline and does not modify media files. App-owned state, including configured Library folders, play history, preferences, and Queue, stays local.

## Settings, folders, and scans

Open **Settings** to manage Library folders. Choose **Add Folder…** to open the desktop folder chooser/portal, or remove a selected folder. Overlapping folders are allowed and do not create duplicate catalog entries.

Select one or more folders and choose **Scan selected**, or choose **Scan all** to scan every configured folder. Scanning is explicit; Kulkán does not watch folders continuously. The dialog shows scanning status and a completion summary or errors. A scan cannot be canceled, and closing Settings does not stop it. A selected scan leaves unselected folders and their catalog entries/history untouched.

Kulkán remembers configured folders and the last scanned catalog locally. Removing a folder does not erase its catalog entries or play history; after a scan confirms those entries are no longer covered, they become inactive. If a removable drive is disconnected or access is revoked, the scan reports an incomplete area and preserves last-known entries there.

Settings also contains the played threshold, play-history controls, state export/import, reset, and **View on startup**. Choose Artists, Albums, or Songs; the initial default is Songs, and a changed choice takes effect on the next launch.

## Browse the catalog

Choose **Artists**, **Albums**, or **Songs** from the left navigation. Songs shows the full scanned Library; Artists groups tracks under the first-billed artist; Albums groups by release title and album-artist credits when present, otherwise by first-billed track artist. Tracks without an album appear under **No Album**. Selecting an artist or Album shows its tracks in the center. Track rows are ordered by artist, release, track number, then title. Inactive entries remain visible as unavailable and cannot be queued. Rows display compact track details; additional metadata such as genre and file location is available in the row tooltip. Missing tags remain missing; Kulkán does not guess metadata from filenames or filesystem timestamps. A plain click selects one row (click it again to deselect); use **Ctrl-click** to toggle additional rows and **Shift-click** to select a range. The same selection and queue actions are available in all track lists.

## Search the Library

Choose **Search** in the left navigation to show search criteria in the center. Basic search ignores case and diacritics while preserving original displayed tags; active filters appear as inline chips. **Advanced search** adds AND-combined artist, album, genre, year, duration, file-location, play-count, and Not-yet-played criteria. Use **is missing** for supported metadata fields. Year, duration (seconds), and play count accept an exact number or an inclusive `min..max` range. Submit the criteria to show paged results in the center; choose **Add Selected** or **Add All Matches** to append results to the Queue in order. The Queue remains visible and unchanged. Search criteria and results are retained when navigating elsewhere and returning to Search. When results are showing, choose **Search** again to edit the existing criteria. **Clear** clears the criteria and results and leaves a blank criteria view.

Supported discovery formats are MP3, PCM WAV, FLAC, Ogg Vorbis, and Ogg Opus. AAC/M4A is not supported and is reported in the scan status rather than silently ignored. A supported file that is malformed or cannot be read is reported as a media error rather than silently treated as a valid track.

## Play a track

Select one or more active catalog rows and choose **Add Selected** or use the icon-based Play/Pause transport to start them. The persistent bottom player bar shows current-track details, elapsed time, duration, and icon buttons for Previous, Stop, Play/Pause, and Next; each button has an accessible name and tooltip. Play/Pause pauses or resumes active playback; clicking the timeline seeks. Previous goes to the prior queue entry when the current track has played for less than two seconds, and otherwise restarts the current track. Playback uses locally available GStreamer decoder and audio-sink plugins and works offline; Kulkán does not download decoders. See [supported formats](../SPEC.md#3-supported-platforms-and-media) for runtime plugin requirements.

## Play history

The **Plays** column shows each entry's local qualifying-play count; zero means **Not-yet-played**. Set the app-wide **Played threshold (%)** from 1–100; it defaults to 75%, and changes apply to new playback attempts. Kulkán counts distinct portions heard during an attempt, so pauses preserve progress, seeks add none, and replayed portions count only once. Use a row's **Clear history** button or **Clear All Play History** to reset counts, including inactive entries.

## Queue

Use **Add Selected** or **Add All Matches** to append entries from search results; outside search, use **Add All Tracks**. Duplicate entries are allowed. The queue rail supports playing an entry, reordering upcoming entries, removing entries, clearing the queue, and changing repeat and shuffle modes. Queue order and repeat/shuffle settings persist across relaunches. On relaunch, the current track is restored from the beginning in a paused state.

## Backup and keyboard shortcuts

Use **Export State…** and **Import State…** in Settings for a local backup or migration. The backup contains Kulkán-owned folders, cached catalog/history, preferences, and queue state; it does not include or modify media files. **Reset App State…** clears the same app-owned data while leaving audio files untouched.

- **Ctrl+F** focuses Basic search.
- **Ctrl+Enter** adds selected results to the Queue.
- **Ctrl+Space** toggles playback.
- **Ctrl+Left** and **Ctrl+Right** go to Previous and Next.
- **Ctrl+S** runs **Scan all** when a text field is not focused.
- **Ctrl+Shift+E** exports state; **Ctrl+Shift+I** imports state.
- **Escape** clears active search criteria/results, or clears track/Queue selection when Search is inactive.

When media metadata does not provide a duration, Kulkán uses GStreamer to discover or derive one and stores it in the catalog. If a file cannot be decoded or its duration cannot be established, the status line reports a playback/discovery error. Playback errors include the GStreamer diagnostic where available.

## Rescans and missing files

- Kulkán reads media; it does not edit, move, rename, or organize audio files.
- A completed scan that confirms a file is gone keeps its catalog record but marks it inactive.
- Removing a Library folder and scanning makes entries no longer covered by any configured folder inactive.
- If a folder or subtree cannot be enumerated, Kulkán reports the affected path and preserves last-known entries there. Reconnect the drive or restore access and scan again.
- Nested symbolic links are skipped. A directory symlink can be scanned when it is explicitly selected as a Library folder.
- Reaching a file through overlapping folders or through a selected directory symlink does not duplicate its canonical path. Separate copies at distinct paths remain separate entries.
- Editing tags refreshes the displayed metadata without changing the entry's identity. Replacing the audio content at the same path creates a new entry and retains the old one as inactive.
- A rename or move preserves identity only when exactly one inactive entry has the same exact encoded audio content. Ambiguous matches and re-encodes start new entries, keeping play history associated with the correct entry.

## Server startup

The GTK client starts a local Rust server. If startup fails, use **Retry** to make one manual replacement attempt or **Quit** to close the application. A second launch activates the running client rather than starting another server.

## Report a problem

Issues are tracked at [GitHub](https://github.com/itzam-na/kulkan/issues). Include the Kulkán version/build, Linux distribution and desktop session, the affected file format, and the exact steps and status/error text. Do not attach personal audio files unless you have made a safe, non-private test sample.
