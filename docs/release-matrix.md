# Kulkán Arch package verification

**Status:** The source-built AUR recipe is prepared but not yet published. Its source URL targets Codeberg tag `v0.1.1`; publish that tag before submitting the recipe to the AUR.

## Required package checks

| Environment | PKGBUILD/.SRCINFO | Clean package build | Install/launch | Audio formats | State and library workflows | Notes/evidence |
| --- | --- | --- | --- | --- | --- | --- |
| Arch Linux x86-64 | Not tested | Not tested | Not tested | Automated `fakesink` decoding only | Not tested | A successful build is not an end-user acceptance pass. |

## Arch verification procedure

1. Confirm `.SRCINFO` matches `PKGBUILD` with `makepkg --printsrcinfo`.
2. Build in a clean x86-64 Arch environment with `makepkg --syncdeps --cleanbuild`; inspect the resulting package with `namcap` when available.
3. Install the package and launch `kulkan`. Confirm both the client and adjacent `kulkan-server` executable start.
4. Add a test media folder, scan it, and verify original tags, canonical locations, inactive entries, and the Library/search/Clear workflow.
5. Play MP3, PCM WAV, FLAC, Ogg Vorbis, and Opus through the host audio sink. Record audible output and GStreamer diagnostics.
6. Verify queue, play history, export/import, server recovery, keyboard, and assistive-technology workflows.
7. Confirm media files are not modified and app-owned state survives restart. Do not attach personal audio as test evidence.

Automated child-server decoding through `fakesink` proves decoder and pipeline behavior only; it does not satisfy package installation, GUI, portal, accessibility, or host audio checks.
