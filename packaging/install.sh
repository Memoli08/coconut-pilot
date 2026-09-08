#!/bin/sh
# Run from the extracted GitHub Release bundle as an ordinary desktop user.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
source="$root/coconut"
if [ ! -x "$source" ]; then
  echo "coconut executable was not found next to install.sh" >&2
  exit 1
fi
destination="${XDG_BIN_HOME:-$HOME/.local/bin}"
mkdir -p "$destination"
install -m755 "$source" "$destination/coconut"
echo "Installed: $destination/coconut"
case ":${PATH:-}:" in
  *":$destination:"*) ;;
  *) echo "Add $destination to PATH, then open a new terminal." ;;
esac
echo "Run: coconut setup"
echo "The setup wizard will ask for sudo only when it installs the keyboard input service."
