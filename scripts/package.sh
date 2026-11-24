#!/usr/bin/env bash
set -euo pipefail

# No-sudo packaging script for BurnBox
# Expects bin/burnbox and bin/burnbox-core to already exist.

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
RELEASE_DIR="$PROJECT_DIR/release"

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
mkdir -p "$RELEASE_DIR"

# --- Build .tar.gz archive ---

# Collect all files for the tar.gz
TGZ_PATH="$RELEASE_DIR/burnbox_$(date +%Y%m%d%H%M).tar.gz"

mkdir -p "$PROJECT_DIR/tmp_package"
cp "$PROJECT_DIR/bin/burnbox" "$PROJECT_DIR/tmp_package/"
cp "$PROJECT_DIR/bin/burnbox-core" "$PROJECT_DIR/tmp_package/"
cp "$PROJECT_DIR/scripts/install.sh" "$PROJECT_DIR/tmp_package/"
cp "$PROJECT_DIR/packaging/burnbox.desktop" "$PROJECT_DIR/tmp_package/"

cd "$PROJECT_DIR/tmp_package"
tar czf "$TGZ_PATH" burnbox install.sh burnbox.desktop
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

# Copy binaries to the package structure
cp "$PROJECT_DIR/bin/burnbox" "$PKG_BUILD_DIR/usr/local/bin/"
cp "$PROJECT_DIR/bin/burnbox-core" "$PKG_BUILD_DIR/usr/local/bin/"

# Copy desktop entry to the package structure
cp "$PROJECT_DIR/packaging/burnbox.desktop" "$PKG_BUILD_DIR/usr/local/share/applications/"

# Create DEBIAN/control
cat > "$PKG_BUILD_DIR/DEBIAN/control" <<EOF
Package: burnbox
Version: 0.7.0
Section: utils
Architecture: amd64
Priority: optional
Maintainer: BurnBox <burnbox@example.com>
Description: Unprivileged namespace sandbox and RAM wipe engine
Homepage: https://github.com/burnbox/burnbox
EOF

# Create postinst script (user-space only, no sudo)
cat > "$PKG_BUILD_DIR/DEBIAN/postinst" <<'POSTINST_EOF'
#!/bin/sh
# postinst: register MIME types and desktop entries for the current user
set -e

# Update desktop database for the user's local application directory
if [ -x /usr/bin/update-desktop-database ]; then
    update-desktop-database "$HOME/.local/share/applications/" 2>/dev/null || true
fi

# Set MIME types as default for BurnBox if not already set
MIME_TYPES="application/pdf;application/zip;application/x-html;text/html;application/vnd.openxmlformats-officedocument.wordprocessingml.document;"
if [ -x /usr/bin/xdg-mime ]; then
    xdg-mime default burnbox.desktop "$MIME_TYPES" 2>/dev/null || true
fi

exit 0
POSTINST_EOF

chmod +x "$PKG_BUILD_DIR/DEBIAN/postinst"

# Build the .deb package
DEB_PATH="$RELEASE_DIR/burnbox_0.7.0_amd64.deb"
dpkg-deb --build "$PKG_BUILD_DIR" "$DEB_PATH"

echo "Created: $DEB_PATH"
echo "Created: $TGZ_PATH"
echo "Packaging complete. Outputs in $RELEASE_DIR/"
echo "💡 Recommendation: Run 'burnbox doctor' to verify your host setup is suitable for sandboxing."