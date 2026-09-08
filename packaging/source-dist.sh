#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)
test -n "$version"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT HUP INT TERM
root="$stage/coconut-pilot-$version"
mkdir -p "$root/.cargo" dist
cp Cargo.toml Cargo.lock LICENSE README.md CHANGELOG.md CONTRIBUTING.md CODE_OF_CONDUCT.md SECURITY.md SUPPORT.md "$root/"
cp -R src tests packaging assets "$root/"
cargo vendor --locked --offline "$root/vendor" > "$root/.cargo/config.toml"
# cargo vendor emits an absolute directory when passed one; make archive relocatable.
python3 - "$root/.cargo/config.toml" <<'PY'
import sys
p=sys.argv[1]
s=open(p).read()
s='\n'.join('directory = "vendor"' if line.startswith('directory = ') else line for line in s.splitlines())+'\n'
open(p,'w').write(s)
PY
tar -C "$stage" -czf "dist/coconut-pilot-$version.tar.gz" "coconut-pilot-$version"
sha256sum "dist/coconut-pilot-$version.tar.gz"
