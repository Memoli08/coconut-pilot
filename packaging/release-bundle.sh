#!/bin/sh
# Produce the portable Linux download used by GitHub Releases.
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
test -n "$version"
cargo build --release --locked
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
root="$stage/coconut-pilot-$version-linux-x86_64"
mkdir -p "$root" dist
install -m755 target/release/coconut "$root/coconut"
install -m755 packaging/install.sh "$root/install.sh"
install -m644 README.md LICENSE CHANGELOG.md CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md SUPPORT.md "$root/"
cp -R assets "$root/"
archive="dist/coconut-pilot-$version-linux-x86_64.tar.gz"
tar -C "$stage" -czf "$archive" "$(basename "$root")"
sha256sum "$archive"
