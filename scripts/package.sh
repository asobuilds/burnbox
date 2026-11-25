#!/usr/bin/env bash
set -euo pipefail

# No-sudo packaging script for BurnBox
# Expects bin/burnbox and bin/burnbox-core to already exist.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RELEASE_DIR="$PROJECT_DIR/release"

# Dynamic target architecture (fall back to amd64 on non-Debian hosts)
ARCH=$(dpkg --print-architecture 2>/dev/null || echo "amd64")

# Extract version dynamically from the built binary (single source of truth)
BURNBOX_VERSION=$("$PROJECT_DIR/bin/burnbox" --version | sed 's/BurnBox v//')
if [ -z "$BURNBOX_VERSION" ]; then
    echo "Error: could not determine version from bin/burnbox --version" >&2
    exit 1
fi

# Verify required binaries exist
if [ ! -f "$PROJECT_DIR/bin/burnbox" ]; then
    echo "Error: bin/burnbox not found. Build it first." >&2
    exit 1
fi

if [ ! -f "$PROJECT_DIR/bin/burnbox-core" ]; then
    echo "Error: bin/burnbox-core not found. Build it first." >&2
    exit 1
fi

# Create release directory
rm -rf "$RELEASE_DIR"/* 2>/dev/null || true
mkdir -p "$RELEASE_DIR"

# --- Build .tar.gz archive ---

# Tarball layout:
#   bin/burnbox
#   bin/burnbox-core
#   install.sh
#   packaging/burnbox.desktop
TGZ_PATH="$RELEASE_DIR/burnbox_${BURNBOX_VERSION}_${ARCH}.tar.gz"

mkdir -p "$PROJECT_DIR/tmp_package/bin"
mkdir -p "$PROJECT_DIR/tmp_package/packaging"
cp "$PROJECT_DIR/bin/burnbox" "$PROJECT_DIR/tmp_package/bin/"
cp "$PROJECT_DIR/bin/burnbox-core" "$PROJECT_DIR/tmp_package/bin/"
cp "$PROJECT_DIR/scripts/install.sh" "$PROJECT_DIR/tmp_package/"
cp "$PROJECT_DIR/packaging/burnbox.desktop" "$PROJECT_DIR/tmp_package/packaging/"

cd "$PROJECT_DIR/tmp_package"
tar czf "$TGZ_PATH" bin/ install.sh packaging/burnbox.desktop
cd "$PROJECT_DIR"
rm -rf "$PROJECT_DIR/tmp_package"

echo "Created: $TGZ_PATH"

# --- Build .deb package ---

# Create package directory structure for dpkg-deb --build
# dpkg-deb --build expects: <dir>/DEBIAN/control, <dir>/usr/local/bin/*, etc.
PKG_BUILD_DIR="$PROJECT_DIR/debian_pkg"
rm -rf "$PKG_BUILD_DIR"
mkdir -p "$PKG_BUILD_DIR/DEBIAN"
mkdir -p "$PKG_BUILD_DIR/usr/local/bin"
mkdir -p "$PKG_BUILD_DIR/usr/local/share/applications"

# Copy binaries to the package structure (/usr/local/bin)
cp "$PROJECT_DIR/bin/burnbox" "$PKG_BUILD_DIR/usr/local/bin/"
cp "$PROJECT_DIR/bin/burnbox-core" "$PKG_BUILD_DIR/usr/local/bin/"

# Copy desktop entry to the package structure (/usr/local/share/applications)
cp "$PROJECT_DIR/packaging/burnbox.desktop" "$PKG_BUILD_DIR/usr/local/share/applications/"

# Create DEBIAN/control using dynamically-determined version
cat > "$PKG_BUILD_DIR/DEBIAN/control" <<EOF
Package: burnbox
Version: $BURNBOX_VERSION
Section: utils
Architecture: $ARCH
Priority: optional
Maintainer: BurnBox <burnbox@example.com>
Description: Unprivileged namespace sandbox and RAM wipe engine
Homepage: https://github.com/burnbox/burnbox
EOF

# Create postinst script (no sudo, runs as root during install)
cat > "$PKG_BUILD_DIR/DEBIAN/postinst" <<'POSTINST_EOF'
#!/bin/sh
# postinst: register MIME types and desktop entries (runs as root during .deb install)
set -e

# Update desktop database for system-wide application directory
if [ -x /usr/bin/update-desktop-database ]; then
    update-desktop-database /usr/local/share/applications/ 2>/dev/null || true
fi

# Set MIME types as default for BurnBox if not already set
MIME_TYPES="application/pdf;application/zip;text/html;application/vnd.openxmlformats-officedocument.wordprocessingml.document;x-scheme-handler/http;x-scheme-handler/https;"
if [ -x /usr/bin/xdg-mime ]; then
    xdg-mime default burnbox.desktop "$MIME_TYPES" 2>/dev/null || true
fi

exit 0
POSTINST_EOF

chmod +x "$PKG_BUILD_DIR/DEBIAN/postinst"

# Build the .deb package using dynamic version
DEB_PATH="$RELEASE_DIR/burnbox_${BURNBOX_VERSION}_${ARCH}.deb"
dpkg-deb --build "$PKG_BUILD_DIR" "$DEB_PATH"

echo "Created: $DEB_PATH"
echo "Created: $TGZ_PATH"
echo "Packaging complete (version $BURNBOX_VERSION). Outputs in $RELEASE_DIR/"
echo "💡 Recommendation: Run 'burnbox doctor' to verify your host setup is suitable for sandboxing."