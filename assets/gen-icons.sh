#!/opt/homebrew/bin/bash
# Generates the state-colored menu-bar icons and the app .icns.
# Requires: rsvg-convert (brew install librsvg), sips + iconutil (built in).
set -euo pipefail

cd "$(dirname "$0")"
mkdir -p menubar build

# --- menu-bar glyph: a tunnel arch on a ground line -------------------------
# One SVG template, recolored per state. Rendered at 44px = 22pt @2x.
menubar_svg() {
  local color="$1"
  cat <<EOF
<svg xmlns="http://www.w3.org/2000/svg" width="44" height="44" viewBox="0 0 44 44">
  <path d="M9 37 L9 22 A13 13 0 0 1 35 22 L35 37" fill="none"
        stroke="${color}" stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/>
  <line x1="5" y1="37" x2="39" y2="37" stroke="${color}" stroke-width="5" stroke-linecap="round"/>
</svg>
EOF
}

declare -A STATES=(
  [running]="#34C759"       # green  — running, read-only (replica)
  [running_write]="#0A84FF" # blue   — running, write (primary)
  [stopped]="#8E8E93"       # gray   — stopped
  [reconnecting]="#FF9F0A"  # amber  — starting / logging in / reconnecting
  [alert]="#FF3B30"         # red    — needs login / error
)

for name in "${!STATES[@]}"; do
  menubar_svg "${STATES[$name]}" | rsvg-convert -w 44 -h 44 -o "menubar/${name}.png"
  echo "menubar/${name}.png"
done

# --- app icon: rounded-rect with a white tunnel arch ------------------------
cat > build/appicon.svg <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" width="1024" height="1024" viewBox="0 0 1024 1024">
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#2E7D5B"/>
      <stop offset="1" stop-color="#16463A"/>
    </linearGradient>
  </defs>
  <rect x="0" y="0" width="1024" height="1024" rx="224" ry="224" fill="url(#bg)"/>
  <g fill="none" stroke="#FFFFFF" stroke-linecap="round" stroke-linejoin="round">
    <path d="M240 780 L240 470 A272 272 0 0 1 784 470 L784 780" stroke-width="70"/>
    <line x1="176" y1="792" x2="848" y2="792" stroke-width="70"/>
    <path d="M392 780 L392 512 A120 120 0 0 1 632 512 L632 780" stroke-width="44" stroke-opacity="0.55"/>
  </g>
</svg>
EOF
rsvg-convert -w 1024 -h 1024 -o build/appicon-1024.png build/appicon.svg

ICONSET=build/AppIcon.iconset
rm -rf "$ICONSET"; mkdir -p "$ICONSET"
gen() { sips -z "$2" "$2" build/appicon-1024.png --out "$ICONSET/$1" >/dev/null; }
gen icon_16x16.png 16
gen icon_16x16@2x.png 32
gen icon_32x32.png 32
gen icon_32x32@2x.png 64
gen icon_128x128.png 128
gen icon_128x128@2x.png 256
gen icon_256x256.png 256
gen icon_256x256@2x.png 512
gen icon_512x512.png 512
gen icon_512x512@2x.png 1024
iconutil -c icns "$ICONSET" -o AppIcon.icns
echo "AppIcon.icns"
