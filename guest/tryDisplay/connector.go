package main

import (
	"os"
	"path/filepath"
	"strings"
)

const drmRoot = "/sys/class/drm"

func windowRule(connector string) (string, display, bool) {
	edid, err := os.ReadFile(filepath.Join(connector, "edid"))
	if err != nil {
		return "", display{}, false
	}
	found, ok := preferredDisplay(edid)
	if !ok {
		return "", display{}, false
	}
	scale := displayScale(found.timing.width, found.timing.height, found.widthMM, found.heightMM)
	rule, ok := displayRule(outputName(connector), found, scale)
	return rule, found, ok
}

func outputName(connector string) string {
	name := filepath.Base(connector)
	if index := strings.Index(name, "-"); index >= 0 {
		name = name[index+1:]
	}
	return name
}

func greeterScale(found display) float64 {
	return displayScale(found.timing.width, found.timing.height, found.widthMM, found.heightMM)
}

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func connectorPath() string {
	entries, err := os.ReadDir(drmRoot)
	if err != nil {
		return ""
	}
	for _, entry := range entries {
		if !strings.Contains(entry.Name(), "-") {
			continue
		}
		path := filepath.Join(drmRoot, entry.Name())
		status, err := os.ReadFile(filepath.Join(path, "status"))
		if err != nil {
			continue
		}
		state := strings.TrimSpace(string(status))
		if state != "connected" && state != "unknown" {
			continue
		}
		edid, err := os.ReadFile(filepath.Join(path, "edid"))
		if err != nil || len(edid) < 72 {
			continue
		}
		return path
	}
	return ""
}
