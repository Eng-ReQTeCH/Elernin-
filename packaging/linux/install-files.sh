#!/usr/bin/env bash
set -euo pipefail

here=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
binary=$1
destination=$2
desktop_id=io.github.Eng_ReQTeCH.Elearnin

install -Dm755 "$binary" "$destination/usr/bin/elearnin"
install -Dm644 "$here/$desktop_id.desktop" "$destination/usr/share/applications/$desktop_id.desktop"
install -Dm644 "$here/$desktop_id.svg" "$destination/usr/share/icons/hicolor/scalable/apps/$desktop_id.svg"
