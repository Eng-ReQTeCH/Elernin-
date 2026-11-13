#!/usr/bin/env bash
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
cd "$here/../.."
[[ $(dpkg --print-architecture) == amd64 ]] || { echo 'An amd64 Debian build environment is required.' >&2; exit 1; }

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$PWD/target}"
cargo build --release --locked --target x86_64-unknown-linux-gnu
version=$(cargo metadata --locked --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
binary="$CARGO_TARGET_DIR/x86_64-unknown-linux-gnu/release/elearnin"
stage=$(mktemp -d)
trap 'rm -rf -- "$stage"' EXIT
chmod 755 "$stage"
bash "$here/install-files.sh" "$binary" "$stage"
strip --strip-unneeded "$stage/usr/bin/elearnin"
desktop-file-validate "$stage/usr/share/applications/"*.desktop

# Resolve linked library minimum versions against the build distribution.
mkdir -p "$stage/debian" "$stage/DEBIAN"
cat > "$stage/debian/control" <<'EOF'
Source: elearnin
Section: education
Priority: optional
Maintainer: Eng-ReQTeCH <212583414+Eng-ReQTeCH@users.noreply.github.com>

Package: elearnin
Architecture: amd64
Description: Offline course library for BUE Learn1
EOF
depends=$(cd "$stage" && dpkg-shlibdeps -O usr/bin/elearnin)
depends=${depends#shlibs:Depends=}
rm "$stage/debian/control"
rmdir "$stage/debian"

cat > "$stage/DEBIAN/control" <<EOF
Package: elearnin
Version: $version
Section: education
Priority: optional
Architecture: amd64
Maintainer: Eng-ReQTeCH <212583414+Eng-ReQTeCH@users.noreply.github.com>
Depends: $depends, ca-certificates, xdg-utils, libx11-6, libx11-xcb1, libxcb1, libxkbcommon0, libxkbcommon-x11-0, libwayland-client0
Recommends: gnome-keyring | keepassxc
Homepage: https://github.com/Eng-ReQTeCH/Elernin-
Description: Offline course library for BUE Learn1
 Downloads lectures and tutorials and opens saved copies from the desktop.
EOF

output="${PACKAGE_OUTPUT:-$PWD/target/installer}"
mkdir -p "$output"
dpkg-deb --root-owner-group --build "$stage" "$output/elearnin_${version}_amd64.deb"
