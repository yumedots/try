package main

import (
	"bytes"
	"crypto/md5"
	"fmt"
	"os"
	"regexp"
	"strings"
	"time"
)

func packagePath() (string, error) {
	path := "/usr/local/lib/try/packages.txt"
	if info, err := os.Stat(path); err == nil && !info.IsDir() {
		return path, nil
	}
	return "", fmt.Errorf("package manifest missing at %s", path)
}

func provisioned(path string) bool {
	data, err := os.ReadFile(path)
	if err != nil {
		return false
	}
	state, err := os.ReadFile("/var/lib/try/provisioned")
	if err != nil {
		return false
	}
	return strings.TrimSpace(string(state)) == fmt.Sprintf("%x", md5.Sum(data))
}

func readPackages(path string) ([]string, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var packages []string
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(strings.SplitN(line, "#", 2)[0])
		if line != "" {
			packages = append(packages, line)
		}
	}
	if len(packages) == 0 {
		return nil, fmt.Errorf("package manifest %s is empty", path)
	}
	return packages, nil
}

func installPackages(manifest string) error {
	packages, err := readPackages(manifest)
	if err != nil {
		return err
	}
	if err := waitForNetwork(); err != nil {
		return err
	}
	if err := configurePacman(); err != nil {
		return err
	}
	if _, err := command("pacman", "-Qq"); err != nil {
		if _, err := command("pacman-db-upgrade"); err != nil {
			return err
		}
	}
	var regular []string
	var hyprland bool
	for _, packageName := range packages {
		if packageName == "hyprland" {
			hyprland = true
		} else {
			regular = append(regular, packageName)
		}
	}
	failed := installPackageBatch(regular)
	for pass := 0; len(failed) > 0 && pass < 2; pass++ {
		failed = installPackagesIndividually(failed)
		if len(failed) > 0 {
			if _, err := command("pacman", "-Syu", "--needed", "--noconfirm", "--noprogressbar"); err != nil {
				logf("pacman system upgrade: %v", err)
			}
		}
	}
	if hyprland {
		if _, err := command("pacman", "-S", "--needed", "--noconfirm", "--noprogressbar", "hyprland"); err != nil {
			if _, fallbackErr := command("pacman", "-S", "--needed", "--noconfirm", "--noprogressbar", "--assume-installed", "libaquamarine.so=13-64", "hyprland"); fallbackErr != nil {
				failed = append(failed, "hyprland")
			}
		}
	}
	if len(failed) > 0 {
		return fmt.Errorf("packages failed: %s", strings.Join(failed, " "))
	}
	return nil
}

func installPackageBatch(packages []string) []string {
	if len(packages) == 0 {
		return nil
	}
	args := append([]string{"-Sy", "--needed", "--noconfirm", "--noprogressbar"}, packages...)
	if _, err := command("pacman", args...); err == nil {
		return nil
	}
	return packages
}

func installPackagesIndividually(packages []string) []string {
	var failed []string
	for _, packageName := range packages {
		if _, err := command("pacman", "-S", "--needed", "--noconfirm", "--noprogressbar", packageName); err != nil {
			failed = append(failed, packageName)
		}
	}
	return failed
}

func waitForNetwork() error {
	for i := 0; i < 30; i++ {
		if _, err := command("ip", "route", "show", "default"); err == nil {
			return nil
		}
		time.Sleep(time.Second)
	}
	return fmt.Errorf("network route did not become available")
}

func pacmanConf(data []byte) []byte {
	parallel := regexp.MustCompile(`(?m)^#?\s*ParallelDownloads\s*=.*$`)
	updated := parallel.ReplaceAll(data, []byte("ParallelDownloads = 10"))
	ignore := regexp.MustCompile(`(?m)^#?\s*IgnorePkg\s*=.*$`)
	replaced := false
	return ignore.ReplaceAllFunc(updated, func(match []byte) []byte {
		if replaced {
			return []byte("# " + string(match))
		}
		replaced = true
		return []byte("IgnorePkg = linux-aarch64")
	})
}

func configurePacman() error {
	path := "/etc/pacman.conf"
	data, err := os.ReadFile(path)
	if err != nil {
		return fmt.Errorf("read pacman.conf: %w", err)
	}
	updated := pacmanConf(data)
	if !bytes.Equal(data, updated) {
		if err := os.WriteFile(path, updated, 0644); err != nil {
			return fmt.Errorf("write pacman.conf: %w", err)
		}
	}
	if _, err := os.Stat("/etc/pacman.d/gnupg/pubring.gpg"); os.IsNotExist(err) {
		if _, err := command("pacman-key", "--init"); err != nil {
			return err
		}
		if _, err := command("pacman-key", "--populate", "archlinuxarm"); err != nil {
			return err
		}
	}
	return nil
}
