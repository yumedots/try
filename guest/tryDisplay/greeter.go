package main

import (
	"fmt"
	"math"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
)

const (
	xftDPI          = "/run/try/xft.dpi"
	xsettingsConfig = "/run/try/xsettingsd.conf"
	xftBaseDPI      = 96
	x11Sockets      = "/tmp/.X11-unix/X*"
	xauthFiles      = "/run/sddm/xauth_*"
	setupHook       = "/usr/local/lib/try/Xsetup"
	setupPath       = "/usr/share/sddm/scripts/Xsetup"
)

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
