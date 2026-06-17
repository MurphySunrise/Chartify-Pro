#!/bin/sh
set -eu

ROOT_DIR=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
APP_NAME="${APP_NAME:-Chartify Pro}"
ZIP_NAME="${ZIP_NAME:-Chartify-Pro-macOS-arm64.zip}"
APP_DIR="$ROOT_DIR/dist/$APP_NAME.app"
CONTENTS_DIR="$APP_DIR/Contents"
MACOS_DIR="$CONTENTS_DIR/MacOS"
RESOURCES_DIR="$CONTENTS_DIR/Resources"

cd "$ROOT_DIR"
cargo build --release

rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR" "$RESOURCES_DIR"

cp "target/release/chartify-pro" "$MACOS_DIR/chartify-pro"
sips -s format icns assets/chartify.png \
    --out "$RESOURCES_DIR/chartify.icns" >/dev/null

cat >"$CONTENTS_DIR/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>Chartify Pro</string>
    <key>CFBundleExecutable</key>
    <string>chartify-pro</string>
    <key>CFBundleIconFile</key>
    <string>chartify</string>
    <key>CFBundleIdentifier</key>
    <string>com.murphysunrise.chartify-pro</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>Chartify Pro</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
PLIST

chmod +x "$MACOS_DIR/chartify-pro"
codesign --force --deep --sign - "$APP_DIR"

rm -f "$ROOT_DIR/dist/$ZIP_NAME"
ditto -c -k --sequesterRsrc --keepParent "$APP_DIR" \
    "$ROOT_DIR/dist/$ZIP_NAME"

echo "Created $APP_DIR"
echo "Created $ROOT_DIR/dist/$ZIP_NAME"
