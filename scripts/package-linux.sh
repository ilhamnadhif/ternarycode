#!/usr/bin/env bash
# Packages the Linux build for this machine's architecture (x86_64 or aarch64) into dist/:
#
#   TernaryCode-<v>-linux-<arch>.tar.gz     any distribution: unpack into ~/.local (see the README)
#   TernaryCode-<v>-linux-<amd64|arm64>.deb Debian, Ubuntu, Mint, Pop!_OS …
#   TernaryCode-<v>-linux-<arch>.rpm        Fedora, RHEL, Rocky, AlmaLinux, openSUSE …
#   TernaryCode-<v>-linux-<arch>.AppImage   most others (Arch, Manjaro …); updates itself
#
#   cargo build --release --locked
#   scripts/package-linux.sh
#
# Needs dpkg-deb, rpmbuild and, for the AppImage, appimagetool (downloaded into target/ unless
# APPIMAGETOOL points to one). Build against the oldest glibc you want to support: CI builds in
# manylinux_2_28 (glibc 2.28: RHEL 8, Debian 10, Ubuntu 18.10 and newer).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *) echo "unsupported architecture: $ARCH" >&2; exit 1 ;;
esac
BIN="${TERNARYCODE_BIN:-$ROOT/target/release/ternarycode}"
HOOK="$(dirname "$BIN")/ternarycode-hook"
OUT="$ROOT/dist"
WORK="$ROOT/target/linux-pkg"
REPO="https://github.com/ilhamnadhif/ternarycode"
SUMMARY="Terminal workspace for many projects and AI coding agents"
DESCRIPTION="TernaryCode is a native, terminal-first workspace written in Rust: every project with its
own terminals, tabs and split panes, AI coding agents (Claude Code, Codex, Gemini CLI, OpenCode)
with their status at a glance, a grid of all projects, a code editor with language servers, git
changes to review, and a viewer for PDF, Word, Excel and PowerPoint files. No account, no telemetry."
NAME="TernaryCode-$VERSION-linux"

[[ -x "$BIN" && -x "$HOOK" ]] || { echo "missing $BIN or $HOOK: run cargo build --release first" >&2; exit 1; }
[[ "$("$BIN" --version)" == "TernaryCode $VERSION" ]] || { echo "$BIN is not TernaryCode $VERSION" >&2; exit 1; }
rm -rf "$WORK"
mkdir -p "$WORK" "$OUT"

echo "== file tree"
TREE="$WORK/tree"
install -Dm755 "$BIN" "$TREE/usr/bin/ternarycode"
# Agents run it from their hooks; the app finds it beside itself.
install -Dm755 "$HOOK" "$TREE/usr/bin/ternarycode-hook"
install -Dm644 packaging/linux/ternarycode.desktop "$TREE/usr/share/applications/ternarycode.desktop"
for icon in packaging/linux/icons/hicolor/*/apps/ternarycode.png; do
  install -Dm644 "$icon" "$TREE/usr/share/icons/${icon#packaging/linux/icons/}"
done
install -Dm644 README.md "$TREE/usr/share/doc/ternarycode/README.md"
install -Dm644 LICENSE "$TREE/usr/share/doc/ternarycode/copyright"

echo "== tar.gz"
# bin/ and share/ at the top, so it unpacks into ~/.local (or /usr/local) as it is.
TAR_DIR="$WORK/ternarycode-$VERSION"
mkdir -p "$TAR_DIR"
cp -a "$TREE/usr/." "$TAR_DIR/"
tar -C "$WORK" -czf "$OUT/$NAME-$ARCH.tar.gz" "ternarycode-$VERSION"

echo "== deb"
DEB="$WORK/deb"
cp -a "$TREE" "$DEB"
mkdir -p "$DEB/DEBIAN"
{
  echo "Package: ternarycode"
  echo "Version: $VERSION"
  echo "Architecture: $DEB_ARCH"
  echo "Maintainer: TernaryCode contributors <ilhamnadhif@users.noreply.github.com>"
  echo "Installed-Size: $(du -sk "$TREE" | cut -f1)"
  # The window system and OpenGL libraries are loaded at run time; desktops have them.
  echo "Depends: libc6 (>= 2.28), libgcc-s1 | libgcc1, libxkbcommon0, libgl1 | libegl1"
  echo "Recommends: git, xdg-utils, xdg-desktop-portal"
  echo "Section: devel"
  echo "Priority: optional"
  echo "Homepage: $REPO"
  echo "Description: $SUMMARY"
  echo "$DESCRIPTION" | sed 's/^/ /'
} > "$DEB/DEBIAN/control"
dpkg-deb --root-owner-group -Zxz --build "$DEB" "$OUT/$NAME-$DEB_ARCH.deb" >/dev/null

echo "== rpm"
RPM="$WORK/rpm"
mkdir -p "$RPM"/{BUILD,RPMS,SOURCES,SPECS,SRPMS}
{
  echo "Name: ternarycode"
  echo "Version: $VERSION"
  echo "Release: 1"
  echo "Summary: $SUMMARY"
  echo "License: MIT"
  echo "URL: $REPO"
  echo "Recommends: git"
  echo "Recommends: xdg-utils"
  # Prebuilt, already stripped binary: no debug package, no rebuilding.
  echo "%define debug_package %{nil}"
  echo "%define __os_install_post %{nil}"
  echo "%define _build_id_links none"
  echo "%description"
  echo "$DESCRIPTION"
  echo "%install"
  echo "mkdir -p %{buildroot}"
  echo "cp -a $TREE/. %{buildroot}/"
  echo "%files"
  echo "/usr/bin/ternarycode"
  echo "/usr/bin/ternarycode-hook"
  echo "/usr/share/applications/ternarycode.desktop"
  echo "/usr/share/icons/hicolor/*/apps/ternarycode.png"
  echo "%doc /usr/share/doc/ternarycode/README.md"
  echo "%license /usr/share/doc/ternarycode/copyright"
} > "$RPM/SPECS/ternarycode.spec"
rpmbuild -bb --quiet --define "_topdir $RPM" --target "$ARCH" "$RPM/SPECS/ternarycode.spec"
cp "$RPM/RPMS/$ARCH/ternarycode-$VERSION-1.$ARCH.rpm" "$OUT/$NAME-$ARCH.rpm"

echo "== AppImage"
APPDIR="$WORK/TernaryCode.AppDir"
cp -a "$TREE" "$APPDIR"
ln -s usr/bin/ternarycode "$APPDIR/AppRun"
cp packaging/linux/ternarycode.desktop "$APPDIR/ternarycode.desktop"
cp packaging/linux/icons/hicolor/256x256/apps/ternarycode.png "$APPDIR/ternarycode.png"
TOOL="${APPIMAGETOOL:-$ROOT/target/appimagetool-$ARCH.AppImage}"
if [[ ! -x "$TOOL" ]]; then
  curl --proto '=https' --tlsv1.2 -fsSL -o "$TOOL" \
    "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage"
  chmod +x "$TOOL"
fi
# Extract-and-run: CI machines have no FUSE.
ARCH="$ARCH" APPIMAGE_EXTRACT_AND_RUN=1 "$TOOL" --no-appstream "$APPDIR" "$OUT/$NAME-$ARCH.AppImage" >/dev/null

ls -la "$OUT"/"$NAME"-*
