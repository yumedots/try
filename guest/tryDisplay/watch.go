package main

import (
	"bufio"
	"os/exec"
	"strings"
	"time"
)

const (
	poll         = 200 * time.Millisecond
	udevCommand  = "udevadm"
	drmSubsystem = "drm"
)

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
