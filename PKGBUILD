# Maintainer: noahlyk <noahlykins@gmail.com>
#
# Local build of the checkout this file sits in (no source download, no AUR): `make pkg` or
# `makepkg -si`. `make pkg` keeps makepkg's work folders out of the repository's own src/.
pkgname=typeladder
pkgver=0.1.0
pkgrel=1
pkgdesc="Adaptive typing test: builds your vocabulary from the commonest word to the rarest, for any keyboard, steno engine or text expander"
arch=('x86_64' 'aarch64')
url="https://github.com/noahlyk/typeladder"
license=('GPL-3.0-or-later')
depends=('gcc-libs')
makedepends=('rust' 'cargo' 'git')
optdepends=('espanso: a text expander the hooks can pause during a test'
            'keymux: a steno engine (the lyk theory exports its dictionary for typeladder)')
options=('!debug' '!lto')
install=typeladder.install

source=()
sha256sums=()

# Local builds from a git checkout report the tag, commit count and hash, the same as `make`:
#   1.2.0+2.gbbd181e, or 1.2.0+2.gbbd181e.dirty with uncommitted changes (build.rs does the same).
# With no `v[0-9]*` tag yet: Cargo.toml's version and the hash, 0.1.0+gbbd181e. No git: pkgver above.
pkgver() {
    cd "$startdir"
    git rev-parse --git-dir >/dev/null 2>&1 || { echo "$pkgver"; return; }

    local version meta=() described
    if described=$(git describe --tags --long --match 'v[0-9]*' 2>/dev/null); then
        local rest="${described%-g*}"
        version="${rest%-*}"
        version="${version#v}"
        local count="${rest##*-}"
        (( count > 0 )) && meta+=("$count.g${described##*-g}")
    else
        version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
        meta+=("g$(git rev-parse --short HEAD)")
    fi
    [ -n "$(git status --porcelain --untracked-files=no)" ] && meta+=(dirty)

    if (( ${#meta[@]} )); then
        local IFS=.
        version+="+${meta[*]}"
    fi
    echo "$version"
}

build() {
    cd "$startdir"
    TYPELADDER_VERSION="$pkgver" cargo build --release --locked
}

package() {
    cd "$startdir"
    install -Dm755 "target/release/typeladder" "$pkgdir/usr/bin/typeladder"
    install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm644 "docs/config.md" "$pkgdir/usr/share/doc/$pkgname/config.md"
    install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
    install -Dm644 "NOTICE" "$pkgdir/usr/share/licenses/$pkgname/NOTICE"
}
