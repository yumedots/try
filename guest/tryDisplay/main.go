package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"syscall"
	"time"
	"unsafe"
)

const (
	readyMarker = "\nTRY_DISPLAY_READY=1\n"
	serialPort  = "/dev/ttyAMA0"
	drmRoot     = "/sys/class/drm"
	poll        = 200 * time.Millisecond

	mmPerInch     = 25.4
	fallbackDPI   = 100
	fallbackScale = 0.0
	okResponse    = "ok"
)

func main() {
	notify()
	connector := ""
	applied := ""
	size := ""
	mode := ""
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
			size = found.timing.size()
			apply(connector, found, rule)
		}
		if current, ok := currentMode(); ok && current != mode {
			mode = current
			tell(fmt.Sprintf("tryDisplay: mode %s\n", mode))
			if mode == size {
				applied = ""
			}
		}
		if session := currentSession(); session != lastSession {
			lastSession = session
			tell(fmt.Sprintf("tryDisplay: session %s\n", session))
			applied = ""
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

func currentMode() (string, bool) {
	if size, ok := debugfsMode(); ok {
		return size, true
	}
	return crtcMode()
}

func debugfsMode() (string, bool) {
	states, err := filepath.Glob("/sys/kernel/debug/dri/*/state")
	if err != nil {
		return "", false
	}
	for _, path := range states {
		data, err := os.ReadFile(path)
		if err != nil {
			continue
		}
		for _, line := range strings.Split(string(data), "\n") {
			_, after, found := strings.Cut(line, "mode: \"")
			if !found {
				continue
			}
			size, _, found := strings.Cut(after, "\"")
			if found {
				return size, true
			}
		}
	}
	return "", false
}

func crtcMode() (string, bool) {
	device, err := os.Open("/dev/dri/card0")
	if err != nil {
		return "", false
	}
	defer device.Close()
	for crtc := uint32(0); crtc < 8; crtc++ {
		state := drmModeCrtc{CrtcID: crtc}
		if _, _, errno := syscall.Syscall(syscall.SYS_IOCTL, device.Fd(), drmModeGetCrtc, uintptr(unsafe.Pointer(&state))); errno != 0 {
			continue
		}
		if state.Mode.Hdisplay > 0 && state.Mode.Vdisplay > 0 {
			return fmt.Sprintf("%dx%d", state.Mode.Hdisplay, state.Mode.Vdisplay), true
		}
	}
	return "", false
}

const drmModeGetCrtc = 0xc05c64a1

type drmModeCrtc struct {
	CrtcID    uint32
	FbID      uint32
	X         uint32
	Y         uint32
	GammaSize uint32
	ModeValid uint32
	Mode      drmModeModeinfo
}

type drmModeModeinfo struct {
	Clock      uint32
	Hdisplay   uint16
	HsyncStart uint16
	HsyncEnd   uint16
	Htotal     uint16
	Hskew      uint16
	Vdisplay   uint16
	VsyncStart uint16
	VsyncEnd   uint16
	Vtotal     uint16
	Vscan      uint16
	Vrefresh   uint32
	Flags      uint32
	Type       uint32
	Name       [32]byte
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
	tell(fmt.Sprintf("tryDisplay: window %s\n", found.timing.size()))
	response, ok := hyprctl("eval", rule)
	if !ok {
		fitConsole(found.timing.width, found.timing.height)
		restartLy()
		return
	}
	reportCompositor(outputName(connector), found.timing.size())
	if response != okResponse {
		tell(fmt.Sprintf("tryDisplay: %s rejected: %s\n", found.timing.size(), response))
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

func fitConsole(width, height int) {
	device, err := os.OpenFile(fbDevice, os.O_RDWR, 0)
	if err != nil {
		return
	}
	defer device.Close()
	variables := fbVar{}
	if !fbRead(device, &variables) {
		return
	}
	if int(variables[fbVisibleWidth]) != width || int(variables[fbVisibleHeight]) != height {
		variables[fbVisibleWidth] = uint32(width)
		variables[fbVisibleHeight] = uint32(height)
		variables[fbOffsetX] = 0
		variables[fbOffsetY] = 0
		if !fbWrite(device, &variables) {
			tell(fmt.Sprintf("tryDisplay: console %s does not fit %dx%d\n", variables.size(), width, height))
			return
		}
		if !fbRead(device, &variables) {
			return
		}
	}
	tell(fmt.Sprintf("tryDisplay: console %s\n", variables.size()))
}

func (v fbVar) size() string {
	return fmt.Sprintf("%dx%d", v[fbVisibleWidth], v[fbVisibleHeight])
}

func fbRead(device *os.File, variables *fbVar) bool {
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, device.Fd(), fbioGetVar, uintptr(unsafe.Pointer(variables)))
	return errno == 0
}

func fbWrite(device *os.File, variables *fbVar) bool {
	_, _, errno := syscall.Syscall(syscall.SYS_IOCTL, device.Fd(), fbioPutVar, uintptr(unsafe.Pointer(variables)))
	return errno == 0
}

type fbVar [fbWords]uint32

const (
	fbVisibleWidth = iota
	fbVisibleHeight
	fbVirtualWidth
	fbVirtualHeight
	fbOffsetX
	fbOffsetY
)

const (
	fbWords    = 40
	fbioGetVar = 0x4600
	fbioPutVar = 0x4601
	fbDevice   = "/dev/fb0"
)

func restartLy() {
	units, err := exec.Command("systemctl", "list-units", "--type=service", "--state=active", "--no-legend", "--plain", "--no-pager", "ly@*.service").Output()
	if err != nil {
		return
	}
	for _, line := range strings.Split(string(units), "\n") {
		fields := strings.Fields(line)
		if len(fields) == 0 || !strings.HasPrefix(fields[0], "ly@") {
			continue
		}
		exec.Command("systemctl", "restart", fields[0]).Run()
		return
	}
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

func notify() {
	tell(readyMarker)
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
