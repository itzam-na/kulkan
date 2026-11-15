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

## Arch Linux (makepkg)

The repository includes a `PKGBUILD` that builds Kulkán from source for x86-64 Arch Linux. Use the Makefile to build and install the package from a checkout:

```sh
sudo pacman -S --needed base-devel git
git clone https://github.com/itzam-na/kulkan.git
cd kulkan
make package-install
```

Run `make package-install` as your regular user, not with `sudo`; it builds the package, syncs dependencies as needed, and uses `sudo` for pacman. Use `make package` to build without installing. The resulting package is built locally from source and is not published as a binary release or AUR package. The PKGBUILD keeps makepkg's work tree and source cache under the ignored `.makepkg-build/` directory.
Remove the installed package with `make package-uninstall`.

## Build from source

Build and launch the release app:

```sh
make release
make run-release
```

For a development build, use `make build` and `make run`.

## Install from source

After `make release`, install binaries and desktop assets under `/usr/local`:

```sh
sudo make install
```

Use `make install-user` to install under `~/.local` without elevated privileges. Remove a source installation with `sudo make uninstall` or `make uninstall-user`, respectively. Override `PREFIX` to choose another prefix; `DESTDIR` is available for staged installs.

After installation, launch the app with:

```sh
make launch
```

## Development

With Rust, GTK 4, and GStreamer development packages installed, run the standard checks through Make:

```sh
make fmt
make test
make clippy
```

Use `make check` to run all three checks, `make gtk-smoke-test` for the isolated headless UI smoke test, `make docs-check` to cross-check the app-flow guide against source and decisions, and `make docs` to generate its HTML/PDF under `target/docs/`. Use `make clean` to remove build artifacts, or `make help` to list workflows. See the [user guide](docs/user-guide.md), [developer guide](docs/development.md), the [app-flow and contributor map](docs/app-flow.md), and product [SPEC](SPEC.md). Domain terms are in [`GLOSSARY.md`](GLOSSARY.md).

## License

Kulkán is licensed under GPL-3.0-or-later. See [`LICENSE`](LICENSE) for the full text.
