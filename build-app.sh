#!/bin/sh
# Builds Quiet Note.app into ./dist
set -e
cd "$(dirname "$0")"
cargo build --release
APP="dist/Quiet Note.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp target/release/quiet-note "$APP/Contents/MacOS/"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Quiet Note</string>
  <key>CFBundleIdentifier</key><string>local.quietnote</string>
  <key>CFBundleExecutable</key><string>quiet-note</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>0.5.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
echo "Готово: $APP"
