# Kulkán user guide

This guide describes the library workflow currently implemented in the local development build. Playback, criteria-based search, queue controls, history, and state export/import are not available yet; their requirements remain in [`SPEC.md`](../SPEC.md) and Codeberg issues.

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

The selected roots are only held for the current server session in this build. Persisting roots and app-owned state across relaunches is tracked separately. The portal must grant access to each selected folder; if a removable drive is disconnected or access is revoked, reconnect it or select the folder again before rescanning.

## Browse the catalog

The catalog displays original tag text when available: title, artist credit, release title, release year, track number, genre, duration, and canonical file location. Missing tags remain missing; Kulkán does not guess metadata from filenames or filesystem timestamps. The current build does not yet offer catalog search or playback.

Supported discovery formats are MP3, PCM WAV, FLAC, Ogg Vorbis, and Ogg Opus. AAC/M4A is not supported. A supported file that is malformed or cannot be read is reported as a media error rather than silently treated as a valid track.

## Rescans and missing files

- Kulkán reads media; it does not edit, move, rename, or organize audio files.
- A completed scan that confirms a file is gone keeps its catalog record but marks it inactive.
- Removing a library root and scanning makes entries known only through that root inactive.
- If a root or subtree cannot be enumerated, Kulkán reports the affected path and preserves last-known entries there. Reconnect the drive or restore access and scan again.
- Nested symbolic links are skipped. A directory symlink can be scanned when it is explicitly selected as a library root.
- Reaching a file through overlapping roots or through a selected directory symlink does not duplicate its canonical path. Separate copies at distinct paths remain separate entries.

## Server startup

The GTK client starts a local Rust server. If startup fails, use **Retry** to make one manual replacement attempt or **Quit** to close the application. A second launch activates the running client rather than starting another server.

## Report a problem

Issues are tracked at [Codeberg](https://codeberg.org/padi/izmusica/issues). Include the Kulkán version/build, Linux distribution and desktop session, the affected file format, and the exact steps and status/error text. Do not attach personal audio files unless you have made a safe, non-private test sample.
