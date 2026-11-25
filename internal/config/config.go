// Package config loads optional BurnBox defaults from the user's config file.
//
// Lookup order:
//  1. $BURNBOX_CONFIG (explicit override, useful for tests and alternate setups)
//  2. ~/.config/burnbox/config.toml
//  3. ~/.config/burnbox/config.json
//
// Both formats accept the keys "mode" (offline|online) and "ram_mb" (positive
// integer). Missing, unreadable, or malformed files fall back to defaults;
// CLI flags always take precedence over whatever is loaded here.
package config

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

// Config holds the user-configurable defaults for `burnbox open`.
type Config struct {
	Mode  string `json:"mode"`
	RamMB int    `json:"ram_mb"`
}

// Default returns the built-in defaults used when no config file is present.
func Default() Config {
	return Config{Mode: "online", RamMB: 512}
}

// Load reads the first available config file and returns normalized defaults.
// It never fails: an absent or broken config simply yields Default().
func Load() Config {
	cfg := Default()

	for _, path := range candidatePaths() {
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		if strings.HasSuffix(path, ".json") {
			// Unmarshal into the pre-filled defaults so absent keys keep theirs.
			_ = json.Unmarshal(data, &cfg)
		} else {
			parseTOML(data, &cfg)
		}
		return cfg.normalized()
	}
	return cfg
}

func candidatePaths() []string {
	if override := os.Getenv("BURNBOX_CONFIG"); override != "" {
		return []string{override}
	}
	home, err := os.UserHomeDir()
	if err != nil || home == "" {
		return nil
	}
	dir := filepath.Join(home, ".config", "burnbox")
	return []string{
		filepath.Join(dir, "config.toml"),
		filepath.Join(dir, "config.json"),
	}
}

// parseTOML handles the small, flat subset of TOML BurnBox needs:
//
//	mode = "offline"
//	ram_mb = 256
//
// Sections, arrays, and other constructs are ignored.
func parseTOML(data []byte, cfg *Config) {
	for _, raw := range strings.Split(string(data), "\n") {
		line := strings.TrimSpace(raw)
		if line == "" || strings.HasPrefix(line, "#") || strings.HasPrefix(line, "[") {
			continue
		}
		parts := strings.SplitN(line, "=", 2)
		if len(parts) != 2 {
			continue
		}
		key := strings.Trim(strings.TrimSpace(parts[0]), `"'`)
		value := strings.TrimSpace(stripInlineComment(parts[1]))
		value = strings.Trim(value, `"'`)

		switch key {
		case "mode":
			cfg.Mode = value
		case "ram_mb":
			if n, err := strconv.Atoi(value); err == nil {
				cfg.RamMB = n
			}
		}
	}
}

func stripInlineComment(value string) string {
	if i := strings.Index(value, "#"); i >= 0 {
		return strings.TrimSpace(value[:i])
	}
	return value
}

// normalized rejects nonsensical values so a bad config can never break a run.
func (c Config) normalized() Config {
	if c.Mode != "offline" && c.Mode != "online" {
		c.Mode = Default().Mode
	}
	if c.RamMB <= 0 {
		c.RamMB = Default().RamMB
	}
	return c
}
