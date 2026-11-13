# Local Flatpak build

The manifest builds the native GTK client and Rust child server from the locked workspace, targets GNOME Platform 51, and grants no network or broad host-filesystem access. Folder selection goes through GTK's native folder chooser/desktop portal; audio uses the host audio socket.

Install `flatpak-builder`, the GNOME 51 runtime/SDK, and the matching Rust stable SDK extension, then build and install into a private local repository:

```sh
flatpak-builder --user --install --force-clean --repo=flatpak/repo flatpak/build flatpak/org.kulkan.Kulkan.yml
flatpak run org.kulkan.Kulkan
```

The Cargo source list is generated from `Cargo.lock` with the upstream `flatpak-cargo-generator.py` tool in the `flatpak-builder-tools` repository. Regenerate `cargo-sources.json` whenever the lockfile changes; see [developer documentation](../docs/development.md) for the complete workflow. `logo.png` is the supplied source artwork; the packaged 512px icon is a resized copy.
