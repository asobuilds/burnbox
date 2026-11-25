package cli

import (
	"os"
	"path/filepath"
	"testing"
)

func TestScanWorkspaces(t *testing.T) {
	root := t.TempDir()
	for _, name := range []string{"burnbox-200", "burnbox-100", "other-300", "burnbox-bad", "burnbox-0"} {
		if err := os.Mkdir(filepath.Join(root, name), 0700); err != nil {
			t.Fatalf("mkdir %s: %v", name, err)
		}
	}
	// Files are not workspaces.
	if err := os.WriteFile(filepath.Join(root, "burnbox-400"), []byte("x"), 0600); err != nil {
		t.Fatalf("writefile: %v", err)
	}

	got, err := scanWorkspaces(root)
	if err != nil {
		t.Fatalf("scanWorkspaces: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("scanWorkspaces returned %d entries, want 2: %+v", len(got), got)
	}
	if got[0].PID != 100 || got[1].PID != 200 {
		t.Fatalf("workspaces not sorted by PID: %+v", got)
	}
	if got[0].Name != "burnbox-100" || got[0].Path != filepath.Join(root, "burnbox-100") {
		t.Fatalf("unexpected workspace metadata: %+v", got[0])
	}
}

func TestParseSessionID(t *testing.T) {
	cases := []struct {
		arg  string
		want int
		ok   bool
	}{
		{"4242", 4242, true},
		{"burnbox-4242", 4242, true},
		{"/dev/shm/burnbox-4242", 4242, true},
		{"burnbox-0", 0, false},
		{"nonsense", 0, false},
		{"", 0, false},
	}
	for _, tc := range cases {
		got, err := parseSessionID(tc.arg, 1)
		if tc.ok && (err != nil || got != tc.want) {
			t.Errorf("parseSessionID(%q) = %d, %v; want %d, nil", tc.arg, got, err, tc.want)
		}
		if !tc.ok && err == nil {
			t.Errorf("parseSessionID(%q) = %d, nil; want error", tc.arg, got)
		}
	}
	// Never allow signaling ourselves.
	if _, err := parseSessionID("1234", 1234); err == nil {
		t.Error("parseSessionID accepted the current PID")
	}
}
