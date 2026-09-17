package main

import (
	"crypto/md5"
	"fmt"
	"os"
	"path/filepath"
)

type runtimeState struct {
	user     guestUser
	config   string
	packages string
	sources  []string
}

func provision(state *runtimeState) ([]step, []action) {
	steps := []step{
		{name: "resolve guest user"},
		{name: "normalize guest ownership"},
		{name: "install guest packages"},
		{name: "verify dotfiles source"},
		{name: "link dotfiles"},
		{name: "configure shell and console"},
		{name: "start guest services"},
		{name: "finish provisioning"},
	}
	actions := []action{
		func() error {
			user, err := resolveUser(1000)
			if err != nil {
				return err
			}
			state.user = user
			state.config = filepath.Join(user.home, ".config")
			return nil
		},
		func() error {
			return normalizeGuestOwnership(state.user)
		},
		func() error {
			packages, err := packagePath()
			if err != nil {
				return err
			}
			state.packages = packages
			if provisioned(packages) {
				logf("packages already provisioned")
				return nil
			}
			return installPackages(packages)
		},
		func() error {
			sources, err := dotfileSources("/usr/local/lib/try/dotfiles")
			if err != nil {
				return err
			}
			state.sources = sources
			return nil
		},
		func() error {
			return linkDotfiles(state.config, state.sources, state.user)
		},
		func() error {
			if err := configureShell(state.user); err != nil {
				return err
			}
			if err := configureConsole(); err != nil {
				return err
			}
			return configureLogin()
		},
		func() error {
			if _, err := command("systemctl", "enable", "--now", "systemd-networkd"); err != nil {
				return err
			}
			if _, err := command("systemctl", "enable", "sshd"); err != nil {
				logf("enable sshd: %v", err)
			}
			if _, err := command("systemctl", "start", "sshd"); err != nil {
				logf("start sshd: %v", err)
			}
			if _, err := command("systemctl", "disable", "--now", "getty@tty1.service"); err != nil {
				logf("disable getty: %v", err)
			}
			if _, err := command("systemctl", "disable", "--now", "ly@tty1.service"); err != nil {
				logf("disable ly: %v", err)
			}
			if _, err := command("systemctl", "enable", "sddm.service"); err != nil {
				return fmt.Errorf("enable sddm: %w", err)
			}
			if _, err := command("systemctl", "restart", "--no-block", "sddm.service"); err != nil {
				return fmt.Errorf("start sddm: %w", err)
			}
			return nil
		},
		func() error {
			data, err := os.ReadFile(state.packages)
			if err != nil {
				return fmt.Errorf("read package manifest: %w", err)
			}
			stamp := fmt.Sprintf("%x\n", md5.Sum(data))
			if err := os.MkdirAll("/var/lib/try", 0755); err != nil {
				return fmt.Errorf("create provisioning state: %w", err)
			}
			if err := os.WriteFile("/var/lib/try/provisioned", []byte(stamp), 0644); err != nil {
				return fmt.Errorf("write provisioning state: %w", err)
			}
			if _, err := command("systemctl", "disable", "firstBoot.service"); err != nil {
				return err
			}
			logf("provisioning succeeded")
			return nil
		},
	}
	return steps, actions
}
