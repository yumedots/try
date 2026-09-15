package main

import (
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
)

func main() {
	notify()
	connector := connectorPath()
	if connector == "" {
		return
	}
	last := ""
	mode := ""
	lastSession := ""
	for {
		size, ok := hostSize(connector)
		if ok && size != last {
			last = size
			apply(connector, size)
		}
		if current, ok := currentMode(); ok && current != mode {
			mode = current
			tell(fmt.Sprintf("tryDisplay: mode %s\n", mode))
		}
		if session := currentSession(); session != lastSession {
			lastSession = session
			tell(fmt.Sprintf("tryDisplay: session %s\n", session))
		}
		time.Sleep(poll)
	}
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

func apply(connector string, size string) {
	os.WriteFile(filepath.Join(connector, "status"), []byte("detect"), 0)
	tell(fmt.Sprintf("tryDisplay: host asked for %s\n", size))
	if !reloadHyprland() {
		restartLy()
	}
}

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

func reloadHyprland() bool {
	sockets := hyprlandSockets()
	if len(sockets) == 0 {
		return false
	}
	command := exec.Command("hyprctl", "reload")
	command.Env = append(os.Environ(),
		"XDG_RUNTIME_DIR="+filepath.Dir(filepath.Dir(filepath.Dir(sockets[0]))),
		"HYPRLAND_INSTANCE_SIGNATURE="+filepath.Base(filepath.Dir(sockets[0])),
	)
	command.Run()
	return true
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
		if err != nil || strings.TrimSpace(string(status)) != "connected" {
			continue
		}
		if _, err := os.Stat(filepath.Join(path, "edid")); err != nil {
			continue
		}
		return path
	}
	return ""
}

func hostSize(connector string) (string, bool) {
	edid, err := os.ReadFile(filepath.Join(connector, "edid"))
	if err != nil || len(edid) < 72 {
		return "", false
	}
	header := []byte{0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00}
	for index, byteValue := range header {
		if edid[index] != byteValue {
			return "", false
		}
	}
	if edid[54] == 0 && edid[55] == 0 {
		return "", false
	}
	width := int(edid[56]) | int(edid[58]&0xf0)<<4
	height := int(edid[59]) | int(edid[61]&0xf0)<<4
	if width == 0 || height == 0 {
		return "", false
	}
	return fmt.Sprintf("%dx%d", width, height), true
}
