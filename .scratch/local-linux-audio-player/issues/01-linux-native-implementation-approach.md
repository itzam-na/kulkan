# Find a Linux-native implementation approach that meets the platform requirements

> Migrated to [Codeberg issue #2](https://codeberg.org/padi/izmusica/issues/2). This is a local snapshot; update the Codeberg issue.

Type: research
Status: resolved

## Question

Which language/runtime, native desktop UI approach, local client-server communication mechanism, audio playback engine, and packaging strategy can meet the requirements together: no Electron, Java, or web runtime; local-only separate processes; common local audio formats; and x86-64 support for Ubuntu, Arch, Mint, Debian, Fedora, and openSUSE? Compare viable approaches using primary sources, identify system-library/runtime dependencies and release baselines, and recommend a practical approach without implementing it.

## Answer

Recommended: Rust native binaries, GTK 4 UI, app-private Unix-domain socket IPC, GStreamer 1.x playback with an explicit common-format plugin set, and Flatpak packaging for x86-64. Flatpak minimizes distro-specific library packaging; GTK portals and host audio permissions must be tested on the target desktops. Findings, alternatives, distro baselines and primary-source citations: [research/implementation-approach.md](../research/implementation-approach.md).

The workspace has no configured Git remote; findings are local and were not pushed.
