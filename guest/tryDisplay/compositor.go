package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const okResponse = "ok"

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
	env, ok := sessionEnv()
	if !ok {
		return "", false
	}
	command := exec.Command("hyprctl", args...)
	command.Env = env
	response, err := command.CombinedOutput()
	return strings.TrimSpace(string(response)), err == nil
}
