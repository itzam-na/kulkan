# Kulkán

Kulkán is a native Linux audio-player project for a listener's local music collection. The product requirements and decisions are in [`SPEC.md`](SPEC.md); domain terms are in [`GLOSSARY.md`](GLOSSARY.md).

## Current state

The GTK client and its Rust child server launch as a local Flatpak. The library foundation supports explicitly selected folders, manual scans, read-only catalog discovery, original metadata display, and safe handling of incomplete scans. Playback, search, queue management, history, and state transfer are still tracked as open implementation issues; this is not yet a complete first release.

## Run the local Flatpak

Build and install the per-user Flatpak using [the Flatpak instructions](flatpak/README.md), then run:

```sh
flatpak run org.kulkan.Kulkan
```

For the first-run library workflow and its current limitations, see the [user guide](docs/user-guide.md). For architecture, tests, and contribution workflows, see the [developer guide](docs/development.md).
