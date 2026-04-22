# Maintainer: Firstpick firstpick1992@proton.me
pkgname=unipack-git
pkgver=0.1.0
pkgrel=1
pkgdesc="Unified terminal UI for pip, npm, bun, cargo, brew, apt, pacman, AUR, rpm, flatpak, and snap (git version)"
arch=('x86_64' 'aarch64')
url="https://github.com/firstp1ck/unipack"
license=('MIT')
depends=('gcc-libs')
optdepends=(
  'python: pip support'
  'python-pip: pip packages'
  'nodejs: npm packages'
  'bun: Bun package manager'
  'rust: cargo / rustup-installed crates'
  'pacman: Arch official packages'
  'yay: AUR packages'
  'paru: AUR packages'
  'flatpak: Flatpak applications'
  'snapd: snap packages'
  'dpkg: Debian/Ubuntu style packages (apt)'
  'rpm: RPM-based distributions'
)
makedepends=('cargo' 'git')
conflicts=('unipack' 'unipack-bin')
provides=('unipack')
# Empty source array - using custom source() function for sparse checkout
source=()
sha256sums=()

# Custom source function to clone with sparse checkout (skip docs-only and dev paths).
fetch_source() {
  cd "$srcdir" || exit 1
  if [ ! -d unipack ]; then
    git clone --filter=blob:none --sparse https://github.com/firstp1ck/unipack.git unipack
  fi
  cd unipack || exit 1
  git pull --tags origin main 2>/dev/null || true
  git sparse-checkout init --no-cone
  git sparse-checkout set '/*' \
    '!/images' '!/dev' '!/.github' '!/.cursor' \
    '!/AGENTS.md' '!/CLAUDE.md' '!/SPEC.md' \
    '!/deny.toml' '!/rustfmt.toml' '!/clippy.toml' '!/install.sh' \
    '!/PKGBUILD-git' '!/.gitattributes' '!/.gitignore'
  git checkout 2>/dev/null || true
}

pkgver() {
  if [ ! -d "$srcdir/unipack" ]; then
    fetch_source >/dev/null 2>&1
  fi
  cd "$srcdir/unipack" || exit 1
  git describe --tags --long --always \
    | sed 's/^v//;s/\([^-]*-g\)/r\1/;s/-/./g'
}

prepare() {
  if [ ! -d "$srcdir/unipack" ]; then
    fetch_source
  fi
  cd "$srcdir/unipack" || exit 1
  export RUSTUP_TOOLCHAIN=stable
  cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"
}

build() {
  cd "$srcdir/unipack" || exit 1

  unset CC CXX AR LD CFLAGS CXXFLAGS LDFLAGS CHOST

  export RUSTUP_TOOLCHAIN=stable
  export CARGO_TARGET_DIR=target
  cargo build --frozen --release --all-features
}

check() {
  cd "$srcdir/unipack" || exit 1
  unset CC CXX AR LD CFLAGS CXXFLAGS LDFLAGS CHOST
  export RUSTUP_TOOLCHAIN=stable
  cargo test --frozen --release --all-features -- --test-threads=1
}

package() {
  cd "$srcdir/unipack" || exit 1
  install -Dm755 "target/release/unipack" "$pkgdir/usr/bin/unipack"
  install -Dm644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 /usr/share/licenses/common/MIT/license "$pkgdir/usr/share/licenses/$pkgname/license"
}
