package main

import (
	"fmt"
	"os"
	"path/filepath"
)

func dotfileSources(root string) ([]string, error) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return nil, fmt.Errorf("read dotfiles source %s: %w; check the submodule is populated", root, err)
	}
	if len(entries) == 0 {
		return nil, fmt.Errorf("dotfiles source %s is empty; check the submodule is populated", root)
	}
	var sources []string
	for _, entry := range entries {
		source := filepath.Join(root, entry.Name())
		if _, err := os.Stat(source); err != nil {
			return nil, fmt.Errorf("dotfiles source missing %s: %w", source, err)
		}
		sources = append(sources, source)
	}
	return sources, nil
}

func linkDotfiles(config string, sources []string, user guestUser) error {
	if err := os.MkdirAll(config, 0755); err != nil {
		return fmt.Errorf("create config directory: %w", err)
	}
	for _, source := range sources {
		destination := filepath.Join(config, filepath.Base(source))
		if _, err := os.Lstat(destination); err == nil {
			if err := os.RemoveAll(destination); err != nil {
				return fmt.Errorf("replace %s: %w", destination, err)
			}
		} else if !os.IsNotExist(err) {
			return fmt.Errorf("inspect %s: %w", destination, err)
		}
		if err := os.Symlink(source, destination); err != nil {
			return fmt.Errorf("link %s to %s: %w", source, destination, err)
		}
		logf("linked %s -> %s", destination, source)
	}
	if err := os.Chown(config, user.uid, user.gid); err != nil {
		logf("chown %s: %v", config, err)
	}
	return nil
}
