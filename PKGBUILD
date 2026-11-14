# Maintainer: Itzamná <itzamna@tenochtitlan.may>
pkgname=kulkan
pkgver=0.1.1
pkgrel=1
pkgdesc="Native GTK music player for a local audio collection"
arch=('x86_64')
url="https://codeberg.org/itzamna/kulkan"
license=('GPL-3.0-or-later')
depends=('gtk4' 'gstreamer' 'gst-plugins-base' 'gst-plugins-good')
makedepends=('cargo' 'git' 'pkgconf')
optdepends=(
  'xdg-desktop-portal-gtk: portal-backed folder chooser'
  'gst-plugin-pipewire: direct PipeWire audio output'
)
source=("git+https://codeberg.org/itzamna/kulkan.git#tag=v${pkgver}")
sha256sums=('SKIP')

prepare() {
  cd "$srcdir/$pkgname"
  export CARGO_HOME="$srcdir/cargo-home"
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "${CARCH}-unknown-linux-gnu"
}

build() {
  cd "$srcdir/$pkgname"
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR=target
  export RUSTUP_TOOLCHAIN=stable
  cargo build --frozen --release -p kulkan --bin kulkan -p kulkan-runtime --bin kulkan-server
}

check() {
  cd "$srcdir/$pkgname"
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR=target
  export RUSTUP_TOOLCHAIN=stable
  cargo test --frozen --workspace
}

package() {
  cd "$srcdir/$pkgname"
  install -Dm0755 target/release/kulkan "$pkgdir/usr/bin/kulkan"
  install -Dm0755 target/release/kulkan-server "$pkgdir/usr/bin/kulkan-server"
  install -Dm0644 data/org.kulkan.Kulkan.desktop "$pkgdir/usr/share/applications/org.kulkan.Kulkan.desktop"
  install -Dm0644 data/icons/512x512/apps/org.kulkan.Kulkan.png "$pkgdir/usr/share/icons/hicolor/512x512/apps/org.kulkan.Kulkan.png"
  install -Dm0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
