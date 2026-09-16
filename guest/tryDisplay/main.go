package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"math"
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

	xftDPI          = "/run/try/xft.dpi"
	xsettingsConfig = "/run/try/xsettingsd.conf"
	xftBaseDPI      = 96
	x11Sockets      = "/tmp/.X11-unix/X*"
	xauthFiles      = "/run/sddm/xauth_*"
	setupHook       = "/usr/local/lib/try/Xsetup"
	setupPath       = "/usr/share/sddm/scripts/Xsetup"

	udevCommand  = "udevadm"
	drmSubsystem = "drm"
)

func main() {
	tell(readyMarker)
	installSetupHook()
	connector := connectorPath()
	applied := ""
	lastSession := ""
	greeter := 1.0
	if _, found, ok := windowRule(connector); ok {
		if scale := greeterScale(found); scale > 0 {
			greeter = scale
		}
	}
	applyGreeterScale(greeter)
	changes := displayChanges()
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
			if scale := greeterScale(found); scale > 0 && scale != greeter {
				greeter = scale
				applyGreeterScale(scale)
			}
		}
		if session := currentSession(); session != lastSession {
			lastSession = session
			applied = ""
			tell(fmt.Sprintf("tryDisplay: session %s\n", session))
		}
		select {
		case <-changes:
		case <-time.After(poll):
		}
	}
}

/*
 * A window resize rewrites the connector's EDID and the kernel announces that
 * as a DRM change event, so wait on those rather than on a clock: the window
 * is followed while it is still being resized instead of after it settles.
 */
func displayChanges() <-chan struct{} {
	changes := make(chan struct{}, 1)

	go func() {
		for {
			watchDisplayChanges(changes)
			time.Sleep(poll)
		}
	}()
	return changes
}

func watchDisplayChanges(changes chan<- struct{}) {
	command := exec.Command(udevCommand, "monitor", "--udev", "--subsystem-match="+drmSubsystem, "--property")
	output, err := command.StdoutPipe()
	if err != nil || command.Start() != nil {
		return
	}
	defer func() {
		command.Process.Kill()
		command.Wait()
	}()

	action, hotplug := "", ""
	reader := bufio.NewReader(output)
	for {
		line, err := reader.ReadString('\n')
		if err != nil {
			return
		}
		line = strings.TrimSpace(line)
		if line == "" {
			if action == "change" && hotplug == "1" {
				select {
				case changes <- struct{}{}:
				default:
				}
			}
			action, hotplug = "", ""
			continue
		}
		switch {
		case strings.HasPrefix(line, "ACTION="):
			action = strings.TrimPrefix(line, "ACTION=")
		case strings.HasPrefix(line, "HOTPLUG="):
			hotplug = strings.TrimPrefix(line, "HOTPLUG=")
		}
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

func greeterScale(found display) float64 {
	return displayScale(found.timing.width, found.timing.height, found.widthMM, found.heightMM)
}

func installSetupHook() {
	script, err := os.ReadFile(setupHook)
	if err != nil || os.MkdirAll(filepath.Dir(setupPath), 0o755) != nil {
		return
	}
	os.WriteFile(setupPath, script, 0o755)
}

func xftDPIValue(scale float64) int {
	return int(math.Round(float64(xftBaseDPI) * scale))
}

func xftResource(scale float64) string {
	return fmt.Sprintf("Xft.dpi: %d\n", xftDPIValue(scale))
}

func xsettingsConfigText(scale float64) string {
	return fmt.Sprintf("Xft/DPI %d\n", xftDPIValue(scale)*1024)
}

func applyGreeterScale(scale float64) {
	if os.MkdirAll(filepath.Dir(xftDPI), 0o755) != nil {
		return
	}
	if os.WriteFile(xftDPI, []byte(xftResource(scale)), 0o644) != nil {
		return
	}
	if os.WriteFile(xsettingsConfig, []byte(xsettingsConfigText(scale)), 0o644) != nil {
		return
	}
	sockets, _ := filepath.Glob(x11Sockets)
	auths, _ := filepath.Glob(xauthFiles)
	for _, socket := range sockets {
		display := ":" + strings.TrimPrefix(filepath.Base(socket), "X")
		for _, auth := range auths {
			if setGreeterDPI(display, auth) {
				tell(fmt.Sprintf("tryDisplay: xft dpi %d on %s\n", xftDPIValue(scale), display))
				return
			}
		}
	}
}

func setGreeterDPI(display, auth string) bool {
	env := append(os.Environ(), "DISPLAY="+display, "XAUTHORITY="+auth)
	command := exec.Command("xrdb", "-merge", xftDPI)
	command.Env = env
	if _, err := command.CombinedOutput(); err != nil {
		return false
	}
	if exec.Command("pgrep", "-x", "xsettingsd").Run() != nil {
		daemon := exec.Command("xsettingsd", "-c", xsettingsConfig)
		daemon.Env = env
		daemon.Start()
	}
	reload := exec.Command("pkill", "-HUP", "-x", "xsettingsd")
	reload.Env = env
	reload.Run()
	return true
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
