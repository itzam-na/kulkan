# Kulkán

Kulkán is a native Linux music player for a personal collection of local audio files. It is built with Rust, GTK 4, and GStreamer and works offline.

## Features

- Browse a paged Library of scanned tracks, including unavailable entries retained from earlier scans.
- Search by text or advanced metadata criteria and queue selected results.
- Play and manage an ordered Queue with repeat, shuffle, and skip-on-failure behavior.
- Track local qualifying-play counts and configure the played threshold.
- Export or import app-owned state without copying or modifying media files.
- Select one row by default; Ctrl-click toggles rows and Shift-click selects a range.

Kulkán reads media metadata and audio content but does not edit, move, rename, or organize media files. Folder selection is explicit, scans are manual, and app-owned state stays local.

## Supported audio formats

MP3, PCM WAV, FLAC, Ogg Vorbis, and Ogg Opus are supported. AAC/M4A is not currently supported.

## Arch Linux / AUR

The repository includes a source-built `kulkan` PKGBUILD and `.SRCINFO` for x86-64 Arch Linux. The `v0.1.1` Codeberg source tag is published; the AUR package submission is still pending. Until then, build and install it from the Codeberg checkout:

```sh
sudo pacman -S --needed base-devel git
git clone https://codeberg.org/itzamna/kulkan.git
cd kulkan
makepkg --syncdeps --cleanbuild
sudo pacman -U ./kulkan-0.1.1-1-x86_64.pkg.tar.zst
```

Run `makepkg` as your regular user, not with `sudo`; `--syncdeps` installs the package's missing build and runtime dependencies. Once the package is submitted to the AUR, it can also be installed with your preferred AUR helper.

## Run

After installation, start the player with:

```sh
kulkan
```

## Development

With Rust, GTK 4, and GStreamer development packages installed:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

See the [user guide](docs/user-guide.md), [developer guide](docs/development.md), [release verification matrix](docs/release-matrix.md), and product [SPEC](SPEC.md). Domain terms are in [`GLOSSARY.md`](GLOSSARY.md).

## License

Kulkán is licensed under GPL-3.0-or-later. See [`LICENSE`](LICENSE) for the full text.
