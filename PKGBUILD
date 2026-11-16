# Maintainer: Itzamná <itzamna@tenochtitlan.may>
pkgname=kulkan
pkgver=0.1.1
pkgrel=2
pkgdesc="Native GTK music player for a local audio collection"
arch=('x86_64')
url="https://github.com/itzam-na/kulkan"
license=('GPL-3.0-or-later')
depends=('gtk4' 'gstreamer' 'gst-plugins-base' 'gst-plugins-good')
makedepends=('cargo' 'git' 'pkgconf')
optdepends=(
  'xdg-desktop-portal-gtk: portal-backed folder chooser'
  'gst-plugin-pipewire: direct PipeWire audio output'
)
_commit=8d3479183f8851dd99c258a5fe5e38edd8c530b9
source=("git+https://github.com/itzam-na/kulkan.git#commit=${_commit}")
sha256sums=('SKIP')

# Keep makepkg's work and source caches out of the application's own src/ tree.
if [[ "${BUILDDIR:-$startdir}" == "$startdir" ]]; then
  BUILDDIR="$startdir/.makepkg-build"
fi
if [[ "${SRCDEST:-$startdir}" == "$startdir" ]]; then
  SRCDEST="$BUILDDIR/sources"
fi
mkdir -p "$BUILDDIR" "$SRCDEST"

prepare() {
  cd "$srcdir/$pkgname"
  export CARGO_HOME="$srcdir/cargo-home"
  export RUSTUP_TOOLCHAIN=stable
  make fetch CARGO_FETCH_FLAGS=--locked TARGET="${CARCH}-unknown-linux-gnu"
}

build() {
  cd "$srcdir/$pkgname"
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR=target
  export RUSTUP_TOOLCHAIN=stable
  make release CARGO_BUILD_FLAGS=--frozen
}

check() {
  cd "$srcdir/$pkgname"
  export CARGO_HOME="$srcdir/cargo-home"
  export CARGO_TARGET_DIR=target
  export RUSTUP_TOOLCHAIN=stable
  make test CARGO_TEST_FLAGS=--frozen
}

package() {
  cd "$srcdir/$pkgname"
  make install DESTDIR="$pkgdir" PREFIX=/usr
}
