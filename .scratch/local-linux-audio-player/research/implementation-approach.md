# Linux-native implementation approach

**Research date:** 2026-11-12
**Scope:** x86-64 Linux desktop client and separate local server; no Electron, Java, or web runtime. This is a recommendation, not an implementation.

## Recommendation

Use **Rust** for both native executables; **GTK 4** (through `gtk4-rs`) for the desktop client; a private **Unix-domain stream socket** for client/server RPC; and **GStreamer 1.x** (through `gstreamer-rs`) in the server. Ship the first release as a **Flatpak** with a GNOME runtime, explicit read-only user media access via GTK/portals, and the required GStreamer runtime/plugins included or provided through a compatible runtime extension. Do not grant network access in the app manifest. Keep the server as a child executable in the same app sandbox; start it on demand and stop it with the client.

This is a native compiled Linux application: GTK widgets render directly through the host display stack; the runtime is GTK/GLib, not a browser, JVM, or other application-level web/VM runtime. The key cost of this choice is that Rust does **not** remove the C-library dependencies of GTK and GStreamer. Flatpak is recommended to version and distribute those dependencies consistently rather than rely on six distro package sets.

## Viable choices and trade-offs

| Concern | Recommended | Viable alternative(s) | Assessment |
|---|---|---|---|
| Language/runtime | Rust native binaries (`x86_64-unknown-linux-gnu`) | C/C++ | Rust’s official Linux GNU target is Tier 1 and is documented for 64-bit Linux with glibc 2.17+ and kernel 3.2+. Rust is ahead-of-time compiled and has no language VM/runtime requirement. C/C++ is equally native and has mature GTK/GStreamer APIs, but offers less compiler-enforced memory/thread safety. Rust still dynamically uses the native GTK/GStreamer stack. [1][2] |
| UI toolkit | GTK 4 with `gtk4-rs` | Qt 6 | GTK is a full widget toolkit with Wayland and X11 support; gtk-rs supplies GTK 4 bindings. Qt is also a sound native choice, especially if KDE conventions dominate, but a Qt stack/runtime would be a different packaging choice. GTK’s native dependencies include GLib/GObject/GIO, Pango, Cairo, GdkPixbuf, Graphene and graphics backends; package these via runtime or distro packages, not as an assumed static Rust executable. [3][4] |
| Local IPC | AF_UNIX stream socket, private to this app | D-Bus session bus | Rust’s Unix standard library directly supports binding/listening on Unix sockets. For one client and its on-demand server, an app-specific socket under the app’s runtime directory avoids a bus service, TCP listener and account/network configuration. Define a versioned, length-delimited protocol; use restrictive socket-directory permissions and validate the peer. D-Bus is appropriate if desktop-wide service discovery or MPRIS integration becomes a requirement, but is extra machinery for private RPC. [5][6] |
| Playback | GStreamer 1.x (`playbin` or a small controlled pipeline) | Separate decoder/output libraries such as Symphonia + CPAL | GStreamer’s playback API is designed for simple playback, handles decode/seek/state/error events, and selects an available audio sink. It has Rust bindings and plugin-based format coverage. A Rust-only decoder/output stack can reduce native framework dependencies, but requires more direct assembly of demux, metadata, seeking, resampling, and host audio integration. GStreamer is the lower-risk all-in-one player engine. [7][8] |
| Distribution | Flatpak x86_64, private or public repository | Native `.deb`, `.rpm`, Arch package | Flatpak provides a distro-independent runtime and allows app dependencies to be bundled; repositories can be private or public. It is one artifact/install mechanism across the requested families, at the cost of a Flatpak prerequisite and explicit sandbox permissions. Native packages integrate closely with system package managers but entail separate metadata, dependency names, testing and release work for Debian/Ubuntu/Mint, Fedora, openSUSE and Arch. [9][10] |

## Audio formats and runtime dependencies

GStreamer is a good fit for **MP3, WAV/PCM, FLAC, Ogg/Vorbis and Opus** as a first supported set: the official plugin reference documents MP3 decoding via `mpg123audiodec` and parsing, WAV parsing, and FLAC, Vorbis and Opus decoders. These formats are provided in GStreamer’s base/good plugin modules (exact split can vary by element); make the required elements part of the tested app runtime, rather than claiming format support merely because a distro may have plugins installed. `playbin` will only decode formats for which elements are available. [11][12][13][14][15]

AAC/M4A and other less-universal formats can be considered later, but should be an explicit decision: their GStreamer support may depend on `libav`/other plugin modules, and the upstream plugin split documentation calls out licensing/distribution considerations for some modules. For offline reliability, the app should not attempt to download a missing decoder at playback time. Test a small fixture corpus for each format and show an actionable unsupported-format error. [16]

For Flatpak, use the GNOME runtime/SDK to provide the GTK stack and use the Freedesktop GStreamer extension point or build GStreamer and the chosen plugin set into the app, pinning compatible versions. The app and server must both find the same plugin registry and libraries inside that runtime. Include the necessary host audio socket permission (Flatpak documents `--socket=pulseaudio` for audio playback); do not grant `--share=network`. The runtime itself is distribution agnostic, with dependencies outside it bundled by the app. [9][17][18]

## IPC, sandbox and local-only behavior

Bind the private socket inside the app’s Flatpak runtime directory (or `$XDG_RUNTIME_DIR` for a native package), not on a TCP port. Have the UI launch the server executable, wait for a successful socket connection, and shut it down cleanly on exit. Make startup idempotent so a stale socket or already-running process is handled safely. Keep the protocol local and small (for example, versioned JSON messages with explicit framing); do not use D-Bus unless a desktop integration feature later requires it. Rust’s `UnixListener` is a first-party standard-library primitive for this. [5]

Flatpak grants no network access by default, but also denies normal host-file access and host audio/display services until permissions are granted. GTK can transparently use portals in a Flatpak; use its native file/directory chooser for user-selected library folders and retain only the access required for rescans. Avoid `--filesystem=host` and other broad read/write grants. Confirm folder permission persistence and mounted removable-drive behavior on each desktop/portal backend during acceptance testing. [18][19]

Flatpak’s security model isolates the app from unrelated host processes. Keep client and server in the **same app sandbox** and treat the server as an app-owned subprocess, not a separately installed host daemon. The D-Bus session bus is filtered by default; a private socket inside the app sandbox avoids requesting broad bus access. Audio and display permissions are still necessary for useful playback and UI. [18]

## Packaging and release baselines

Flatpak’s Freedesktop runtime is explicitly a cross-distribution base; it has a two-year support window per branch. GNOME runtime branches are synced with GNOME and are usually supported for one year. Plan regular runtime-branch upgrades and test against the active branch; the application should not tie its own API compatibility to a particular host distro release. [9]

The official distro and release sources establish these **current test baselines** as of the research date. They are recommended CI/manual test targets, not a claim that older versions are unsupported by Flatpak:

| Distro family | Current baseline to test | Flatpak availability / relevant caveat |
|---|---|---|
| Ubuntu | 26.04 LTS (amd64) | Flatpak install instructions cover Ubuntu 18.10 and newer. Ubuntu’s App Center does not install Flatpaks; users need CLI or another Flatpak-enabled software center. [20][21] |
| Linux Mint | Mint 22.3 (Ubuntu Noble base) | Flatpak is built in from Mint 18.3 onward; current Mint releases are supported through April 2029. [22][23] |
| Debian | Debian 13 “trixie” (current 13.7; amd64) | Debian’s official Flatpak setup says its package is available from Debian 10 onward. Debian 13 has five years of support, with amd64 supported. [24][25] |
| Arch Linux | Current x86_64 rolling release | Flatpak is available in Arch’s official Extra repository; Arch is rolling, so test current updates rather than a numbered release. [26][27] |
| Fedora | Fedora Linux 45 | Fedora 45 was announced GA on 2026-11-03; Fedora Workstation includes Flatpak by default. Fedora releases receive approximately 13 months of support. [28][29] |
| openSUSE | Leap 16.1 and current Tumbleweed | Flatpak is available in maintained Leap and Tumbleweed repositories. Leap 16.1 requires x86_64-v2 (not all older x86-64 CPUs), so document that additional CPU baseline if claiming Leap 16.1 support. Tumbleweed is rolling. [30][31] |

Flatpak setup is not the same as a promise that every historic distro release has the same portal, audio and graphics behavior. State support against the tested matrix and the Flatpak/runtime prerequisites, and revisit it at release time. In particular, the user-selected-folder portal and audio socket path need real distro/desktop tests. When installed and dependencies are present, the runtime can work offline; install and update operations naturally require a configured source unless a bundle is supplied.

If users require system package-manager integration, add distro-native packages as a later channel. For that channel, build separate `.deb`, `.rpm` and Arch package definitions with GTK 4, GStreamer core, audio sink and the exact decoder plugin dependencies declared explicitly; do not assume identical library/plugin versions across releases. GStreamer’s Linux install guidance itself lists different package names for Fedora and Debian/Ubuntu. [8]

## Conclusion

The most practical low-maintenance combination is **Rust + GTK 4 + Unix-domain RPC + GStreamer + Flatpak**. It satisfies the native/no-web-runtime and separate-process requirements, keeps IPC local, supports common local audio formats when the plugin set is shipped, and avoids six parallel system-library packaging tracks. The important acceptance risks are portal persistence for selected folders, audio routing on the host, codec/plugin inclusion and runtime-branch upgrades. Test those before making a broad distro support claim; do not implement them as assumptions.

## Primary sources

1. Rust, [platform support](https://doc.rust-lang.org/rustc/platform-support.html) (`x86_64-unknown-linux-gnu`).
2. Rust, [The Rust Programming Language: compilation and execution](https://doc.rust-lang.org/book/ch01-02-hello-world.html); [Rust language overview](https://www.rust-lang.org/).
3. GTK, [Linux installation](https://gtk.org/docs/installations/linux/); [GTK and Rust bindings](https://www.gtk.org/docs/language-bindings/rust).
4. GTK, [overview and dependencies](https://docs.gtk.org/gtk4/overview.html); [building GTK](https://docs.gtk.org/gtk4/building.html).
5. Rust standard library, [`UnixListener`](https://doc.rust-lang.org/std/os/unix/net/struct.UnixListener.html).
6. Freedesktop.org, [D-Bus Specification](https://dbus.freedesktop.org/doc/dbus-specification.html).
7. GStreamer, [playback components](https://gstreamer.freedesktop.org/documentation/application-development/highlevel/playback-components.html); [`playbin`](https://gstreamer.freedesktop.org/documentation/playback/playbin.html).
8. GStreamer, [installing on Linux](https://gstreamer.freedesktop.org/documentation/installing/on-linux.html); [Rust bindings installation](https://gstreamer.freedesktop.org/documentation/rust/stable/latest/docs/gstreamer/index.html).
9. Flatpak, [available runtimes](https://docs.flatpak.org/en/latest/available-runtimes.html).
10. Flatpak, [basic concepts: runtimes and bundled libraries](https://docs.flatpak.org/en/latest/basic-concepts.html); [repositories](https://docs.flatpak.org/en/latest/repositories.html).
11. GStreamer, [`mpg123audiodec`](https://gstreamer.freedesktop.org/documentation/mpg123/index.html); [`mpegaudioparse`](https://gstreamer.freedesktop.org/documentation/audioparsers/mpegaudioparse.html).
12. GStreamer, [`wavparse`](https://gstreamer.freedesktop.org/documentation/wavparse/index.html); [`flacdec`](https://gstreamer.freedesktop.org/documentation/flac/flacdec.html).
13. GStreamer, [`vorbisdec`](https://gstreamer.freedesktop.org/documentation/vorbis/vorbisdec.html).
14. GStreamer, [`opusdec`](https://gstreamer.freedesktop.org/documentation/opus/opusdec.html).
15. GStreamer, [plugin reference](https://gstreamer.freedesktop.org/documentation/plugins_doc.html).
16. GStreamer, [plugin set split and distribution considerations](https://gstreamer.freedesktop.org/documentation/additional/splitup.html); [FFmpeg/libav plugin](https://gstreamer.freedesktop.org/documentation/libav/index.html).
17. Flatpak, [extensions and runtime extension points](https://docs.flatpak.org/en/latest/extension.html).
18. Flatpak, [sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html).
19. Flatpak, [GTK portal support](https://docs.flatpak.org/en/latest/portals.html); [desktop integration](https://docs.flatpak.org/en/latest/desktop-integration.html).
20. Flathub, [Ubuntu setup](https://flathub.org/setup/Ubuntu).
21. Canonical, [Ubuntu release cycle](https://ubuntu.com/about/release-cycle).
22. Flathub, [Linux Mint setup](https://flatpak.org/setup/Linux%20Mint).
23. Linux Mint, [supported releases](https://linuxmint.com/download_all.php).
24. Debian, [current stable release](https://www.debian.org/releases/stable).
25. Flathub, [Debian setup](https://flatpak.org/setup/Debian).
26. Arch Linux, [official Flatpak package (x86_64)](https://archlinux.org/packages/extra/x86_64/flatpak/).
27. ArchWiki, [Arch Linux and rolling-release model](https://wiki.archlinux.org/title/Arch_Linux).
28. Fedora Project, [Fedora Linux 45 announcement](https://fedoramagazine.org/announcing-fedora-linux-45/).
29. Fedora Docs, [release lifecycle](https://docs.fedoraproject.org/en-US/releases/lifecycle/); [Flatpak setup](https://flatpak.org/setup/Fedora).
30. Flathub, [openSUSE setup](https://flathub.org/setup/openSUSE).
31. openSUSE, [Leap 16.1 features/release baseline](https://en.opensuse.org/Features_16.1); [release roadmap](https://en.opensuse.org/openSUSE:Roadmap).

## Workspace limitation

This workspace has no configured Git remote (`git remote -v` is empty). The findings remain local; no push was attempted.
