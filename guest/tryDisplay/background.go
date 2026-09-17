package main

import (
	"fmt"
	"image"
	_ "image/jpeg"
	_ "image/png"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"
)

const (
	awwwCommand     = "awww"
	backgroundKey   = "misc:background_color"
	toneSamples     = 32
	backgroundTries = 15
)

func applyBackgroundTone() {
	for attempt := 0; attempt < backgroundTries; attempt++ {
		tone, ok := wallpaperTone()
		if ok {
			if _, ok := hyprctl("keyword", backgroundKey, tone); ok {
				tell(fmt.Sprintf("tryDisplay: background %s\n", tone))
			}
			return
		}
		time.Sleep(poll)
	}
}

func wallpaperTone() (string, bool) {
	response, ok := awww("query")
	if !ok {
		return "", false
	}
	path := wallpaperFrom(response)
	if path == "" {
		return "", false
	}
	file, err := os.Open(path)
	if err != nil {
		return "", false
	}
	defer file.Close()
	return toneOf(file)
}

func wallpaperFrom(query string) string {
	for _, line := range strings.Split(query, "\n") {
		value, found := strings.CutPrefix(strings.TrimSpace(line), "image:")
		if !found {
			continue
		}
		return strings.Trim(strings.TrimSpace(value), `"`)
	}
	return ""
}

func toneOf(reader io.Reader) (string, bool) {
	img, _, err := image.Decode(reader)
	if err != nil {
		return "", false
	}
	bounds := img.Bounds()
	stepX := max(1, bounds.Dx()/toneSamples)
	stepY := max(1, bounds.Dy()/toneSamples)
	var red, green, blue, count uint64
	for y := bounds.Min.Y; y < bounds.Max.Y; y += stepY {
		for x := bounds.Min.X; x < bounds.Max.X; x += stepX {
			r, g, b, _ := img.At(x, y).RGBA()
			red, green, blue = red+uint64(r>>8), green+uint64(g>>8), blue+uint64(b>>8)
			count++
		}
	}
	if count == 0 {
		return "", false
	}
	return fmt.Sprintf("0x%02x%02x%02x", red/count, green/count, blue/count), true
}

func awww(args ...string) (string, bool) {
	env, ok := sessionEnv()
	if !ok {
		return "", false
	}
	command := exec.Command(awwwCommand, args...)
	command.Env = env
	response, err := command.CombinedOutput()
	return strings.TrimSpace(string(response)), err == nil
}

func sessionEnv() ([]string, bool) {
	sockets := hyprlandSockets()
	if len(sockets) == 0 {
		return nil, false
	}
	runtime := filepath.Dir(filepath.Dir(filepath.Dir(sockets[0])))
	env := append(os.Environ(),
		"XDG_RUNTIME_DIR="+runtime,
		"HYPRLAND_INSTANCE_SIGNATURE="+filepath.Base(filepath.Dir(sockets[0])),
	)
	if displays, _ := filepath.Glob(filepath.Join(runtime, "wayland-*")); len(displays) > 0 {
		env = append(env, "WAYLAND_DISPLAY="+filepath.Base(displays[0]))
	}
	return env, true
}
