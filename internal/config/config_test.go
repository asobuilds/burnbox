package config

import (
	"os"
	"path/filepath"
	"testing"
)

func writeConfig(t *testing.T, name, content string) string {
	t.Helper()
	dir := t.TempDir()
	path := filepath.Join(dir, name)
	if err := os.WriteFile(path, []byte(content), 0600); err != nil {
		t.Fatalf("write config: %v", err)
	}
	t.Setenv("BURNBOX_CONFIG", path)
	return path
}

func TestLoadDefaultsWhenMissing(t *testing.T) {
	t.Setenv("BURNBOX_CONFIG", filepath.Join(t.TempDir(), "missing.toml"))
	got := Load()
	if got.Mode != "online" || got.RamMB != 512 {
		t.Fatalf("Load() = %+v, want online/512", got)
	}
}

func TestLoadTOML(t *testing.T) {
	writeConfig(t, "config.toml", "# BurnBox defaults\n mode = \"offline\" # stay local\nram_mb = 256\n")
	got := Load()
	if got.Mode != "offline" || got.RamMB != 256 {
		t.Fatalf("Load() = %+v, want offline/256", got)
	}
}

func TestLoadJSON(t *testing.T) {
	writeConfig(t, "config.json", `{"mode":"offline","ram_mb":128}`)
	got := Load()
	if got.Mode != "offline" || got.RamMB != 128 {
		t.Fatalf("Load() = %+v, want offline/128", got)
	}
}

func TestLoadNormalizesBadValues(t *testing.T) {
	writeConfig(t, "config.toml", "mode = \"banana\"\nram_mb = -5\n")
	got := Load()
	if got.Mode != "online" || got.RamMB != 512 {
		t.Fatalf("Load() = %+v, want sanitized online/512", got)
	}
}
