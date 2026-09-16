package main

import "testing"

const monitorsJSON = `[{"id":0,"name":"HDMI-A-1","width":1920,"height":1080,"scale":1},
{"id":1,"name":"Virtual-1","width":2304,"height":1440,"scale":2.00}]`

func TestCompositorFollowsTheWindow(t *testing.T) {
	filled := compositorFill([]byte(monitorsJSON), "Virtual-1", "2304x1440")
	if filled != "tryDisplay: compositor 2304x1440 scale 2\n" {
		t.Errorf("window sized output reported as %q", filled)
	}
}

func TestCompositorBehindTheWindow(t *testing.T) {
	behind := compositorFill([]byte(monitorsJSON), "Virtual-1", "2560x1440")
	if behind != "tryDisplay: compositor 2304x1440 does not fill 2560x1440\n" {
		t.Errorf("unfilled output reported as %q", behind)
	}
}

func TestCompositorUnknownOutput(t *testing.T) {
	if message := compositorFill([]byte(monitorsJSON), "DP-1", "2304x1440"); message != "" {
		t.Errorf("an output with no monitor entry reported %q", message)
	}
	if message := compositorFill([]byte("not json"), "Virtual-1", "2304x1440"); message != "" {
		t.Errorf("unparseable output reported %q", message)
	}
}
