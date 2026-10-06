#!/bin/sh
# Build Labelwerk.app (release, ad-hoc signed) into dist/, plus the `labelwerk` CLI next to it.
set -eu
cd "$(dirname "$0")/.."
cargo build --release -p labelwerk-app -p labelwerk-cli
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=dist/Labelwerk.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/labelwerk-app "$APP/Contents/MacOS/Labelwerk"
cp target/release/labelwerk dist/labelwerk
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Labelwerk</string>
  <key>CFBundleDisplayName</key><string>Labelwerk</string>
  <key>CFBundleIdentifier</key><string>nrw.neuhaus.labelwerk</string>
  <key>CFBundleExecutable</key><string>Labelwerk</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - "$APP"
echo "built $APP"
