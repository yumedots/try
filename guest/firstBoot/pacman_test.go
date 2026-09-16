package main

import (
	"strings"
	"testing"
)

func TestPacmanConfKeepsTheKernelTheHostBoots(t *testing.T) {
	config := "[options]\n#ParallelDownloads = 5\n#IgnorePkg =\nIgnorePkg = linux-aarch64-headers\n[core]\n"
	updated := string(pacmanConf([]byte(config)))
	if strings.Count(updated, "IgnorePkg = linux-aarch64\n") != 1 {
		t.Fatalf("expected one kernel IgnorePkg line, got %q", updated)
	}
	if strings.Contains(updated, "\nIgnorePkg = linux-aarch64-headers") {
		t.Fatalf("expected the older IgnorePkg line to be commented out, got %q", updated)
	}
	for _, kept := range []string{"[options]", "[core]", "ParallelDownloads = 10"} {
		if !strings.Contains(updated, kept) {
			t.Fatalf("expected %q to be kept, got %q", kept, updated)
		}
	}
}
