package config

import (
	"os"
	"path/filepath"
	"testing"
)

func TestTVPlusConfig(t *testing.T) {
	tests := []struct {
		name     string
		file     string
		contents string
		want     bool
	}{
		{"json true", "c.json", `{"tvplus": true}`, true},
		{"yaml true", "c.yml", "tvplus: true\n", true},
		{"toml true", "c.toml", "tvplus = true\n", true},
		{"explicit false", "c.yml", "tvplus: false\n", false},
		{"omitted", "c.json", `{"drm": true}`, false},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			path := filepath.Join(t.TempDir(), tt.file)
			if err := os.WriteFile(path, []byte(tt.contents), 0o600); err != nil {
				t.Fatal(err)
			}
			var cfg JioTVConfig
			if err := cfg.Load(path); err != nil {
				t.Fatal(err)
			}
			if cfg.TVPlus != tt.want {
				t.Errorf("TVPlus = %v, want %v", cfg.TVPlus, tt.want)
			}
		})
	}
}

func TestTVPlusConfigFromEnv(t *testing.T) {
	t.Chdir(t.TempDir()) // no config file in the working directory
	t.Setenv("JIOTV_TVPLUS", "true")
	var cfg JioTVConfig
	if err := cfg.Load(""); err != nil {
		t.Fatal(err)
	}
	if !cfg.TVPlus {
		t.Error("JIOTV_TVPLUS=true not applied")
	}
}
