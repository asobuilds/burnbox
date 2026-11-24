#!/usr/bin/env bash
set -euo pipefail

# Zero-sudo setup script for BurnBox
# Installs binaries and registers 'Open With' context menu handler
# Expects tarball-extracted layout: bin/burnbox, bin/burnbox-core

BURNBOX_BIN="bin/burnbox"
BURNBOX_CORE="bin/burnbox-core"

# Check that binaries exist (relative to current dir)
if [ ! -f "$BURNBOX_BIN" ] || [ ! -f "$BURNBOX_CORE" ]; then
    echo "Error: bin/burnbox and bin/burnbox-core must exist in the current directory." >&2
    exit 1
fi

# Create directories if they don't exist
mkdir -p "$HOME/.local/bin"
mkdir -p "$HOME/.local/share/applications"

# Copy binaries to ~/.local/bin/
cp "$BURNBOX_BIN" "$HOME/.local/bin/burnbox"
cp "$BURNBOX_CORE" "$HOME/.local/bin/burnbox-core"

# Copy desktop entry to ~/.local/share/applications/
cp packaging/burnbox.desktop "$HOME/.local/share/applications/burnbox.desktop"

# Register as default handler for supported MIME types
MIME_TYPES="application/pdf;application/zip;text/html;application/vnd.openxmlformats-officedocument.wordprocessingml.document;x-scheme-handler/http;x-scheme-handler/https;"
xdg-mime default burnbox.desktop "$MIME_TYPES" 2>/dev/null || true

# Update desktop database
update-desktop-database "$HOME/.local/share/applications/" 2>/dev/null || true

echo "BurnBox installed to ~/.local/bin and registered as a safe 'Open With' handler."
echo "💡 Recommendation: Run 'burnbox doctor' to verify your host setup is suitable for sandboxing."