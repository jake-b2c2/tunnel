#!/usr/bin/env bash
# Build the release binary and assemble dist/Tunnel.app.
# Icons are committed to the repo, so this needs only Rust — no librsvg.
set -euo pipefail
cd "$(dirname "$0")"

for icon in assets/menubar/running.png assets/menubar/stopped.png \
            assets/menubar/reconnecting.png assets/menubar/alert.png \
            assets/AppIcon.icns; do
    if [[ ! -f "$icon" ]]; then
        echo "error: missing $icon" >&2
        echo "Regenerate icons with: make icons  (requires librsvg: brew install librsvg)" >&2
        exit 1
    fi
done

echo "==> Building release binary"
cargo build --release

APP="dist/Tunnel.app"
echo "==> Assembling $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp Info.plist "$APP/Contents/Info.plist"
cp target/release/tunnel "$APP/Contents/MacOS/tunnel"
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

# Ad-hoc codesign so Gatekeeper is happy with a locally built app.
codesign --force --deep --sign - "$APP" 2>/dev/null || \
    echo "   (codesign skipped — app still runs locally)"

echo "==> Built $APP"
