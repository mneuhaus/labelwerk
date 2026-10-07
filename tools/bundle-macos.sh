#!/bin/sh
# Build Labelwerk.app (release, ad-hoc signed) into dist/, plus the `labelwerk` CLI next to it.
# --universal: Apple Silicon + Intel in one binary, and the release archives in dist/.
set -eu
cd "$(dirname "$0")/.."
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
APP=dist/Labelwerk.app
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

if [ "${1:-}" = "--universal" ]; then
    for target in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --release --target "$target" -p labelwerk-app -p labelwerk-cli
    done
    for bin in labelwerk-app labelwerk; do
        lipo -create -output "dist/$bin" \
            "target/aarch64-apple-darwin/release/$bin" "target/x86_64-apple-darwin/release/$bin"
    done
    mv dist/labelwerk-app "$APP/Contents/MacOS/Labelwerk"
else
    cargo build --release -p labelwerk-app -p labelwerk-cli
    cp target/release/labelwerk-app "$APP/Contents/MacOS/Labelwerk"
    cp target/release/labelwerk dist/labelwerk
fi

cp crates/labelwerk-app/assets/Labelwerk.icns "$APP/Contents/Resources/Labelwerk.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Labelwerk</string>
  <key>CFBundleDisplayName</key><string>Labelwerk</string>
  <key>CFBundleIdentifier</key><string>nrw.neuhaus.labelwerk</string>
  <key>CFBundleExecutable</key><string>Labelwerk</string>
  <key>CFBundleIconFile</key><string>Labelwerk</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>${VERSION%%-*}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
codesign --force --sign - dist/labelwerk
codesign --force --sign - "$APP"
echo "built $APP ($VERSION)"

if [ "${1:-}" = "--universal" ]; then
    rm -f dist/*.zip dist/*.tar.gz
    ditto -c -k --keepParent "$APP" "dist/Labelwerk-$VERSION-macos.zip"
    tar -czf "dist/labelwerk-cli-$VERSION-macos.tar.gz" -C dist labelwerk
    (cd dist && shasum -a 256 ./*.zip ./*.tar.gz > "SHA256SUMS-$VERSION.txt")
    ls -la dist
fi
