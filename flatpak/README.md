# Local Flatpak build

The manifest builds both native processes from the locked Rust workspace and grants no network or broad host-filesystem access. Folder access is requested through GTK portals; audio uses the host audio socket.

Install `flatpak-builder`, the GNOME 49 runtime/SDK, and the matching Rust stable SDK extension, then build and install into a private local repository:

```sh
flatpak-builder --user --install --force-clean --repo=repo build org.kulkan.Kulkan.yml
flatpak run org.kulkan.Kulkan
```

The Cargo source list is generated from `Cargo.lock` with the upstream `flatpak-cargo-generator.py` tool in `flatpak/flatpak-builder-tools`. Regenerate `cargo-sources.json` whenever the lockfile changes. `logo.png` is the supplied source artwork; the packaged 512px icon is a resized copy.
