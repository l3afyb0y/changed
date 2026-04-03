pkgname=changed
pkgver=0.3.0
pkgrel=1
pkgdesc="Lightweight system tuning changelog daemon for Arch Linux"
arch=('x86_64')
url=""
license=('MIT')
depends=('systemd')
makedepends=('cargo')
options=('!lto')

build() {
  cd "$startdir"
  cargo build --frozen --release
}

check() {
  cd "$startdir"
  cargo test --frozen
}

package() {
  cd "$startdir"

  install -Dm755 "target/release/changed" "$pkgdir/usr/bin/changed"
  install -Dm755 "target/release/changedd" "$pkgdir/usr/bin/changedd"

  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 "README.md" "$pkgdir/usr/share/doc/$pkgname/README.md"
  install -Dm644 "docs/changed.1.md" "$pkgdir/usr/share/doc/$pkgname/changed.1.md"
  install -Dm644 "docs/help-text.md" "$pkgdir/usr/share/doc/$pkgname/help-text.md"
  install -Dm644 "docs/categories.md" "$pkgdir/usr/share/doc/$pkgname/categories.md"
  install -Dm644 "docs/scope-model.md" "$pkgdir/usr/share/doc/$pkgname/scope-model.md"
  install -Dm644 "example-log.md" "$pkgdir/usr/share/doc/$pkgname/example-log.md"

  install -Dm644 "packaging/systemd/system/changedd.service" \
    "$pkgdir/usr/lib/systemd/system/changedd.service"
  install -Dm644 "packaging/systemd/user/changedd.service" \
    "$pkgdir/usr/lib/systemd/user/changedd.service"
}
