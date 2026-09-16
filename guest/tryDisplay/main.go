package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	readyMarker = "\nTRY_DISPLAY_READY=1\n"
	serialPort  = "/dev/ttyAMA0"
	drmRoot     = "/sys/class/drm"
	poll        = 200 * time.Millisecond
	okResponse  = "ok"
)

func main() {
	tell(readyMarker)
	connector := ""
	applied := ""
	lastSession := ""
	for {
		if connector == "" || !exists(filepath.Join(connector, "status")) {
			connector = connectorPath()
			if connector == "" {
				time.Sleep(poll)
				continue
			}
			tell(fmt.Sprintf("tryDisplay: connector %s\n", filepath.Base(connector)))
			applied = ""
		}
		if rule, found, ok := windowRule(connector); ok && rule != applied {
			applied = rule
			apply(connector, found, rule)
		}
		if session := currentSession(); session != lastSession {
			lastSession = session
			applied = ""
			tell(fmt.Sprintf("tryDisplay: session %s\n", session))
		}
		time.Sleep(poll)
	}
}

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

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func currentSession() string {
	if len(hyprlandSockets()) > 0 {
		return "wayland"
	}
	return "tty"
}

func hyprlandSockets() []string {
	sockets, _ := filepath.Glob("/run/user/*/hypr/*/.socket.sock")
	return sockets
}

func apply(connector string, found display, rule string) {
	os.WriteFile(filepath.Join(connector, "status"), []byte("detect"), 0)
	tell(fmt.Sprintf("tryDisplay: window %s\n", found.timing.rate()))
	response, ok := hyprctl("eval", rule)
	if !ok {
		return
	}
	reportCompositor(outputName(connector), found.timing.size())
	if response != okResponse {
		tell(fmt.Sprintf("tryDisplay: %s rejected: %s\n", found.timing.rate(), response))
	}
}

func reportCompositor(output, want string) {
	response, ok := hyprctl("-j", "monitors")
	if !ok {
		return
	}
	if message := compositorFill([]byte(response), output, want); message != "" {
		tell(message)
	}
}

func compositorFill(response []byte, output, want string) string {
	var monitors []struct {
		Name   string  `json:"name"`
		Width  int     `json:"width"`
		Height int     `json:"height"`
		Scale  float64 `json:"scale"`
	}
	if json.Unmarshal(response, &monitors) != nil {
		return ""
	}
	for _, monitor := range monitors {
		if monitor.Name != output {
			continue
		}
		size := fmt.Sprintf("%dx%d", monitor.Width, monitor.Height)
		if size == want {
			return fmt.Sprintf("tryDisplay: compositor %s scale %g\n", size, monitor.Scale)
		}
		return fmt.Sprintf("tryDisplay: compositor %s does not fill %s\n", size, want)
	}
	return ""
}

func hyprctl(args ...string) (string, bool) {
	sockets := hyprlandSockets()
	if len(sockets) == 0 {
		return "", false
	}
	command := exec.Command("hyprctl", args...)
	command.Env = append(os.Environ(),
		"XDG_RUNTIME_DIR="+filepath.Dir(filepath.Dir(filepath.Dir(sockets[0]))),
		"HYPRLAND_INSTANCE_SIGNATURE="+filepath.Base(filepath.Dir(sockets[0])),
	)
	response, err := command.CombinedOutput()
	return strings.TrimSpace(string(response)), err == nil
}

func tell(message string) {
	device, err := os.OpenFile(serialPort, os.O_WRONLY, 0)
	if err != nil {
		return
	}
	defer device.Close()
	device.WriteString(message)
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
