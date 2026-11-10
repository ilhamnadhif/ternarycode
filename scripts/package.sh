#!/usr/bin/env bash
# Builds TernaryCode.app (with ternarycode-hook beside the app binary), signs it, and makes a DMG:
#   dist/TernaryCode-<version>.dmg
#   dist/TernaryCode-<version>.zip    the app alone
#
#   scripts/package.sh                                   # ad-hoc signed, this Mac's architecture
#   TERNARYCODE_ARCHS="aarch64 x86_64" scripts/package.sh # universal (needs both Rust targets)
#   TERNARYCODE_SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#   TERNARYCODE_NOTARY_PROFILE=ternarycode-notary scripts/package.sh
#                                                        # Developer ID signing + notarization
# TERNARYCODE_NOTARY_PROFILE is a notarytool keychain profile, created once with:
#   xcrun notarytool store-credentials ternarycode-notary --apple-id <id> --team-id <TEAMID>
set -euo pipefail
cd "$(dirname "$0")/.."

APP_NAME="TernaryCode"
BINS_IN_APP="ternarycode ternarycode-hook"
BUNDLE_ID="dev.ternarycode.app"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
HOST_ARCH="$(uname -m | sed 's/arm64/aarch64/')"
ARCHS="${TERNARYCODE_ARCHS:-$HOST_ARCH}"
DIST="dist"
# A `.noindex` folder: Spotlight skips it, so macOS doesn't open this copy instead of the
# installed one.
APP="$DIST/app.noindex/$APP_NAME.app"
DMG="$DIST/$APP_NAME-$VERSION.dmg"
ZIP="$DIST/$APP_NAME-$VERSION.zip"
export MACOSX_DEPLOYMENT_TARGET=12.0

echo "==> Building $APP_NAME $VERSION for: $ARCHS"
for arch in $ARCHS; do
    cargo build --release --locked --target "$arch-apple-darwin" -p ternarycode -p tc-agent
done

rm -rf "$APP" "$DMG" "$ZIP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for bin in $BINS_IN_APP; do
    parts=()
    for arch in $ARCHS; do parts+=("target/$arch-apple-darwin/release/$bin"); done
    if [ "${#parts[@]}" -gt 1 ]; then
        lipo -create -output "$APP/Contents/MacOS/$bin" "${parts[@]}"
    else
        cp "${parts[0]}" "$APP/Contents/MacOS/$bin"
    fi
done
cp assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>en</string>
    <key>CFBundleLocalizations</key><array><string>en</string><string>id</string></array>
    <key>CFBundleDisplayName</key><string>$APP_NAME</string>
    <key>CFBundleExecutable</key><string>ternarycode</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>$APP_NAME</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
    <key>NSHumanReadableCopyright</key><string>© 2026 ilhamnadhif. MIT License.</string>
</dict>
</plist>
PLIST

echo "==> Signing"
if [ -n "${TERNARYCODE_SIGN_IDENTITY:-}" ]; then
    codesign --force --timestamp --options runtime --sign "$TERNARYCODE_SIGN_IDENTITY" "$APP/Contents/MacOS/ternarycode-hook"
    codesign --force --timestamp --options runtime --sign "$TERNARYCODE_SIGN_IDENTITY" "$APP"
else
    echo "    (ad-hoc: set TERNARYCODE_SIGN_IDENTITY for a Developer ID signature)"
    codesign --force --sign - "$APP/Contents/MacOS/ternarycode-hook"
    codesign --force --sign - "$APP"
fi
codesign --verify --strict --verbose=1 "$APP"

# A .zip can't be stapled, and the installer and the in-app updater take it: the app itself is
# notarized and stapled first, so the .zip and the DMG carry its ticket.
if [ -n "${TERNARYCODE_NOTARY_PROFILE:-}" ]; then
    echo "==> Notarizing the app"
    SUBMIT="$(mktemp -d)"
    ditto -c -k --sequesterRsrc --keepParent "$APP" "$SUBMIT/$APP_NAME.zip"
    xcrun notarytool submit "$SUBMIT/$APP_NAME.zip" --keychain-profile "$TERNARYCODE_NOTARY_PROFILE" --wait
    rm -rf "$SUBMIT"
    xcrun stapler staple "$APP"
fi

ditto -c -k --sequesterRsrc --keepParent "$APP" "$ZIP"

echo "==> Creating DMG"
STAGE="$(mktemp -d)"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
hdiutil create -quiet -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

if [ -n "${TERNARYCODE_NOTARY_PROFILE:-}" ]; then
    echo "==> Notarizing the DMG"
    xcrun notarytool submit "$DMG" --keychain-profile "$TERNARYCODE_NOTARY_PROFILE" --wait
    xcrun stapler staple "$DMG"
else
    echo "    (not notarized: set TERNARYCODE_SIGN_IDENTITY and TERNARYCODE_NOTARY_PROFILE)"
fi
echo "==> Done"
ls -lh "$DMG" "$ZIP"
