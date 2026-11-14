# Kulkán release verification matrix

**Status:** Release acceptance is incomplete. Do not infer a platform pass from a build or from the automated GStreamer `fakesink` tests.

For each environment, record the OS image/version, CPU model, Flatpak runtime and application commit, desktop/session, portal backend, audio sink, date, and tester. A platform row passes only after the installed Flatpak launches, folder access survives relaunch/removable-media testing, host audio is audible, and every supported format plays.

| Required x86-64 baseline | Install/launch | Folder portal and relaunch | Removable media | Host audio | MP3/WAV/FLAC/Vorbis/Opus | Notes/evidence |
| --- | --- | --- | --- | --- | --- | --- |
| Ubuntu 26.04 LTS | Not tested | Not tested | Not tested | Not tested | Not tested | |
| Linux Mint 22.3 | Not tested | Not tested | Not tested | Not tested | Not tested | |
| Debian 13.7 | Not tested | Not tested | Not tested | Not tested | Not tested | |
| Current Arch rolling | Flatpak build passed; install/launch not tested | Not tested | Not tested | Not tested | Automated `fakesink` decoding only | Current development host; this is not a platform acceptance pass. |
| Fedora 45 | Not tested | Not tested | Not tested | Not tested | Not tested | |
| openSUSE Tumbleweed | Not tested | Not tested | Not tested | Not tested | Not tested | |
| openSUSE Leap 16.1 | Not tested | Not tested | Not tested | Not tested | Not tested | Verify the documented x86-64-v2 CPU requirement. |

## Per-platform procedure

1. Install the Flatpak from the project-local repository using the documented build/install command, then launch Kulkán.
2. Add a test media folder through the desktop folder chooser, scan it, and verify the original tags and file locations.
3. Relaunch; confirm the cached catalog and selected roots return without an implicit scan. Rescan and verify portal grants still work.
4. Disconnect/reconnect a removable-media root or revoke/restore its grant; confirm incomplete scans preserve last-known entries.
5. Play MP3, PCM WAV, FLAC, Ogg Vorbis, and Opus through the host audio sink. Record audible output and any plugin/runtime diagnostics.
6. Verify queue, search, history, backup/restore, recovery, keyboard, and assistive-technology workflows on that desktop. Record any unverified manual interaction explicitly.
7. Confirm media files were not modified. Do not attach personal recordings as test evidence.

Automated child-server decoding through `fakesink` proves decoder/pipeline behavior only; it does not satisfy the host audio or distro checks above. Detailed developer commands and the installed-format test workflow are in [`development.md`](development.md), and the release capstone is tracked by [issue #28](https://codeberg.org/padi/izmusica/issues/28).
