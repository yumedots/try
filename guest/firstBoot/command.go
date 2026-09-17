package main

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"os/exec"
	"strings"
)

func interactive() bool {
	info, err := os.Stdin.Stat()
	return err == nil && info.Mode()&os.ModeCharDevice != 0
}

func command(name string, args ...string) (string, error) {
	cmd := exec.Command(name, args...)
	cmd.Stdin = os.Stdin
	var output bytes.Buffer
	if interactive() {
		cmd.Stdout = &output
		cmd.Stderr = &output
	} else {
		stream := io.MultiWriter(&output, os.Stderr)
		cmd.Stdout = stream
		cmd.Stderr = stream
		fmt.Fprintf(os.Stderr, "[try] %s %s\n", name, strings.Join(args, " "))
	}
	err := cmd.Run()
	text := strings.TrimSpace(output.String())
	if text != "" {
		logf("%s %s\n%s", name, strings.Join(args, " "), text)
	}
	if err != nil {
		if text == "" {
			return "", fmt.Errorf("%s: %w", name, err)
		}
		return text, fmt.Errorf("%s: %w: %s", name, err, lastLines(text, 8))
	}
	return text, nil
}

func lastLines(value string, count int) string {
	lines := strings.Split(value, "\n")
	if len(lines) <= count {
		return value
	}
	return strings.Join(lines[len(lines)-count:], "\n")
}
