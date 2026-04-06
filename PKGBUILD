# Hand-maintained PKGBUILD for Arch packaging.
pkgname=changed
pkgver=0.5.9
pkgrel=1
pkgdesc="Lightweight system tuning changelog daemon for Arch Linux"
arch=('x86_64')
url="https://github.com/l3afyb0y/changed"
license=('MIT')
depends=('systemd')
makedepends=('cargo' 'scdoc')
options=('!lto')

build() {
  cd "$startdir"
  cargo build --locked --release
  scdoc < "docs/changed.1.scd" > "docs/changed.1"
}

check() {
  cd "$startdir"
  cargo test --locked
}

package() {
  cd "$startdir"

  install -Dm755 "build/cargo/release/changed" "$pkgdir/usr/bin/changed"
  install -Dm755 "build/cargo/release/changedd" "$pkgdir/usr/bin/changedd"

  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 "docs/changed.1" "$pkgdir/usr/share/man/man1/changed.1"
  install -Dm644 "docs/changed.1.md" "$pkgdir/usr/share/doc/$pkgname/changed.1.md"
  install -Dm644 "example-log.md" "$pkgdir/usr/share/doc/$pkgname/example-log.md"

  install -Dm644 "packaging/systemd/system/changedd.service" \
    "$pkgdir/usr/lib/systemd/system/changedd.service"
  install -Dm644 "packaging/systemd/user/changedd.service" \
    "$pkgdir/usr/lib/systemd/user/changedd.service"
}
