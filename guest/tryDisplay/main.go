package main

import (
	"fmt"
	"path/filepath"
	"time"
)

const readyMarker = "\nTRY_DISPLAY_READY=1\n"

func main() {
	tell(readyMarker)
	installSetupHook()
	connector := connectorPath()
	applied := ""
	lastSession := ""
	background := false
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
			if session == "wayland" && !background {
				background = true
				go applyBackgroundTone()
			}
		}
		select {
		case <-changes:
		case <-time.After(poll):
		}
	}
}
