package cli

import (
	"os"
	"strings"
	"testing"
)

// Desktop file validation test
// The desktop file is at <project-root>/packaging/burnbox.desktop
func TestDesktopFileValidation(t *testing.T) {
	// Desktop file absolute path
	desktopPath := "/home/student/projects/burnbox/packaging/burnbox.desktop"
	
	data, err := os.ReadFile(desktopPath)
	if err != nil {
		t.Fatalf("failed to read desktop file %s: %v", desktopPath, err)
	}

	content := string(data)

	// Check Exec line - desktop entries use %F (uppercase) for file operations
	execLine := "Exec=burnbox open %F"
	if !strings.Contains(content, execLine) {
		t.Fatalf("desktop file missing Exec line: %s", execLine)
	}

	// Check MimeType line contains expected types (all on one line separated by semicolons)
	mimeLine := "MimeType=application/pdf;application/zip;text/html;application/vnd.openxmlformats-officedocument.wordprocessingml.document;x-scheme-handler/http;x-scheme-handler/https;"
	if !strings.Contains(content, mimeLine) {
		t.Fatalf("desktop file missing expected MimeType line")
	}

	// Verify the desktop file is parseable by checking key fields
	lines := strings.Split(content, "\n")
	inExec := false
	inMime := false
	for _, line := range lines {
		switch {
		case strings.HasPrefix(line, "Exec="):
			inExec = true
		case strings.HasPrefix(line, "MimeType="):
			inMime = true
		}
	}
	if !inExec {
		t.Fatal("desktop file does not contain Exec= field")
	}
	if !inMime {
		t.Fatal("desktop file does not contain MimeType= field")
	}
}

// TestIsNativeELF verifies that the snap-shim detection works:
// - A real ELF binary is recognized
// - A #! shell-script wrapper (snap shim) is rejected
func TestIsNativeELF(t *testing.T) {
	// Real ELF magic bytes
	elfData := []byte{0x7f, 'E', 'L', 'F', 0x02, 0x01, 0x01, 0x00}
	if err := os.WriteFile("/tmp/bb_test_real_elf", elfData, 0755); err != nil {
		t.Fatalf("failed to write test ELF file: %v", err)
	}
	defer os.Remove("/tmp/bb_test_real_elf")

	if !isNativeELF("/tmp/bb_test_real_elf") {
		t.Error("isNativeELF: expected real ELF to be recognized as native")
	}

	// Snap shim shell wrapper
	shimData := []byte("#!/bin/sh\nexec snap-confine run /usr/lib/firefox/firefox \"$@\"\n")
	if err := os.WriteFile("/tmp/bb_test_snap_shim", shimData, 0755); err != nil {
		t.Fatalf("failed to write test snap shim: %v", err)
	}
	defer os.Remove("/tmp/bb_test_snap_shim")

	if isNativeELF("/tmp/bb_test_snap_shim") {
		t.Error("isNativeELF: expected snap shim wrapper to be rejected")
	}

	// Nonexistent file
	if isNativeELF("/tmp/bb_does_not_exist") {
		t.Error("isNativeELF: expected nonexistent file to be rejected")
	}

	// Empty file
	if err := os.WriteFile("/tmp/bb_test_empty", nil, 0755); err != nil {
		t.Fatalf("failed to write empty test file: %v", err)
	}
	defer os.Remove("/tmp/bb_test_empty")

	if isNativeELF("/tmp/bb_test_empty") {
		t.Error("isNativeELF: expected empty file to be rejected")
	}
}