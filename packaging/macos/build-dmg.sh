#!/usr/bin/env bash
# Builds Bridge.app and a .dmg from one (universal) binary. Run on a Mac:
#   packaging/macos/build-dmg.sh path/to/bridge-app 0.3.0 out/Bridge-0.3.0-macos.dmg
# The app is signed "ad hoc" (no Apple developer account): macOS runs it, but the first start needs
# right-click > Open, see the README.
set -euo pipefail

BIN=${1:?path to the bridge-app binary}
VERSION=${2:?version, for example 0.3.0}
OUT=${3:?path of the .dmg to write}
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
APP="$WORK/dmg/Bridge.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

cp "$BIN" "$APP/Contents/MacOS/bridge-app"
chmod +x "$APP/Contents/MacOS/bridge-app"
cp "$ROOT/packaging/icons/bridge.icns" "$APP/Contents/Resources/bridge.icns"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist.in" > "$APP/Contents/Info.plist"
cp "$ROOT/LICENSE" "$WORK/dmg/LICENSE.txt"

codesign --force --deep --sign - "$APP"
codesign --verify --deep --strict "$APP"

ln -s /Applications "$WORK/dmg/Applications"
mkdir -p "$(dirname "$OUT")"
rm -f "$OUT"
hdiutil create -volname "Bridge $VERSION" -srcfolder "$WORK/dmg" -ov -format UDZO "$OUT"
echo "wrote $OUT"
