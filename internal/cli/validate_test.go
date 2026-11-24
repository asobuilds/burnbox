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