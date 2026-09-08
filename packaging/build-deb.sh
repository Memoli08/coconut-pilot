#!/bin/sh
# Run on the Debian/Ubuntu version targeted by this package, with libglib2.0-dev.
set -eu
cd "$(dirname "$0")/.."
for tool in dpkg dpkg-deb dpkg-shlibdeps; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Debian package build requires $tool; run this script on Debian or Ubuntu." >&2
    exit 1
  fi
done
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
test -n "$version"
cargo build --release --locked
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
mkdir -p "$stage/DEBIAN" "$stage/usr/bin" "$stage/usr/share/doc/coconut-pilot" dist
install -m755 target/release/coconut "$stage/usr/bin/coconut"
cp LICENSE README.md "$stage/usr/share/doc/coconut-pilot/"
# dpkg-shlibdeps resolves the platform's GLib/libc package names, including t64.
mkdir -p "$stage/debian"
printf 'Source: coconut-pilot\nSection: utils\nPriority: optional\nMaintainer: Coconut Pilot contributors\nStandards-Version: 4.6.2\n\nPackage: coconut-pilot\nArchitecture: any\nDescription: Copilot key launcher\n' > "$stage/debian/control"
deps=$(cd "$stage" && dpkg-shlibdeps -O -eusr/bin/coconut | sed -n 's/^shlibs:Depends=//p')
rm -r "$stage/debian"
cat > "$stage/DEBIAN/control" <<CONTROL
Package: coconut-pilot
Version: $version-1
Section: utils
Priority: optional
Architecture: $(dpkg --print-architecture)
Maintainer: Coconut Pilot contributors
Depends: $deps, systemd, sudo
Recommends: libnotify-bin
Description: Configure your Copilot key using an interactive terminal interface
 Open installed applications, websites, files, terminals and commands.
CONTROL
dpkg-deb --root-owner-group --build "$stage" "dist/coconut-pilot_${version}-1_$(dpkg --print-architecture).deb"
