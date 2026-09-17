package main

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
)

func configureShell(user guestUser) error {
	if err := os.WriteFile(filepath.Join(user.home, ".zshenv"), []byte("export ZDOTDIR=\"$HOME/.config/zsh\"\n"), 0600); err != nil {
		return fmt.Errorf("write .zshenv: %w", err)
	}
	if err := os.Chown(filepath.Join(user.home, ".zshenv"), user.uid, user.gid); err != nil {
		logf("chown .zshenv: %v", err)
	}
	if _, err := os.Stat(filepath.Join(user.home, ".oh-my-zsh")); os.IsNotExist(err) {
		if _, err := command("git", "clone", "--depth", "1", "https://github.com/ohmyzsh/ohmyzsh", filepath.Join(user.home, ".oh-my-zsh")); err != nil {
			logf("oh-my-zsh unavailable: %v", err)
		}
	}
	if _, err := os.Stat("/usr/bin/zsh"); err == nil && user.shell != "/usr/bin/zsh" {
		if _, err := command("chsh", "-s", "/usr/bin/zsh", user.name); err != nil {
			logf("set zsh shell: %v", err)
		}
	}
	return nil
}

func configureConsole() error {
	if err := os.WriteFile("/etc/locale.conf", []byte("LANG=C.UTF-8\n"), 0644); err != nil {
		return fmt.Errorf("write locale.conf: %w", err)
	}
	matches, _ := filepath.Glob("/usr/share/kbd/consolefonts/ter-232n.psf*")
	if len(matches) > 0 {
		if err := os.WriteFile("/etc/vconsole.conf", []byte("FONT=ter-232n\n"), 0644); err != nil {
			return fmt.Errorf("write vconsole.conf: %w", err)
		}
		if _, err := command("setfont", "ter-232n"); err != nil {
			logf("set console font: %v", err)
		}
	}
	if _, err := command("unicode_start"); err != nil {
		return fmt.Errorf("enable unicode console: %w", err)
	}
	return nil
}

func configureLogin() error {
	loginDefsPath := "/etc/login.defs"
	data, err := os.ReadFile(loginDefsPath)
	if err != nil {
		return fmt.Errorf("read %s: %w", loginDefsPath, err)
	}
	line := []byte("UID_MIN 1000")
	pattern := regexp.MustCompile(`(?m)^\s*UID_MIN\s+.*$`)
	if pattern.Match(data) {
		data = pattern.ReplaceAll(data, line)
	}
	if err := os.WriteFile(loginDefsPath, data, 0644); err != nil {
		return fmt.Errorf("write %s: %w", loginDefsPath, err)
	}
	return nil
}
