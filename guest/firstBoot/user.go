package main

import (
	"fmt"
	"os"
	"path/filepath"
	"strconv"
	"strings"
)

type guestUser struct {
	name  string
	home  string
	uid   int
	gid   int
	shell string
}

func resolveUser(uid int) (guestUser, error) {
	user, err := lookupUser(strconv.Itoa(uid))
	if err == nil {
		return user, nil
	}
	if uid != 1000 {
		if _, lookupErr := lookupUser("alarm"); lookupErr == nil {
			if _, err := command("usermod", "-u", strconv.Itoa(uid), "alarm"); err != nil {
				return guestUser{}, err
			}
			if _, err := command("groupmod", "-g", strconv.Itoa(uid), "alarm"); err != nil {
				return guestUser{}, err
			}
			return lookupUser(strconv.Itoa(uid))
		}
	}
	return guestUser{}, fmt.Errorf("no guest user with uid %d", uid)
}

func normalizeGuestOwnership(user guestUser) error {
	for _, path := range []string{"/", "/home"} {
		if _, err := os.Lstat(path); os.IsNotExist(err) {
			continue
		}
		if err := os.Chown(path, 0, 0); err != nil {
			return fmt.Errorf("chown %s: %w", path, err)
		}
	}
	for _, path := range []string{"/boot", "/etc", "/lib", "/lib64", "/opt", "/root", "/sbin", "/usr", "/var"} {
		if _, err := os.Lstat(path); os.IsNotExist(err) {
			continue
		}
		if err := filepath.Walk(path, func(current string, info os.FileInfo, err error) error {
			if err != nil {
				return err
			}
			if info.Mode()&os.ModeSymlink != 0 {
				return os.Lchown(current, 0, 0)
			}
			return os.Chown(current, 0, 0)
		}); err != nil {
			return fmt.Errorf("chown %s: %w", path, err)
		}
	}
	if err := os.Chown(user.home, user.uid, user.gid); err != nil {
		return fmt.Errorf("chown %s: %w", user.home, err)
	}
	return nil
}

func lookupUser(identifier string) (guestUser, error) {
	data, err := os.ReadFile("/etc/passwd")
	if err != nil {
		return guestUser{}, fmt.Errorf("read passwd: %w", err)
	}
	for _, line := range strings.Split(string(data), "\n") {
		fields := strings.Split(line, ":")
		if len(fields) < 7 || (identifier != fields[0] && identifier != fields[2]) {
			continue
		}
		uid, _ := strconv.Atoi(fields[2])
		gid, _ := strconv.Atoi(fields[3])
		return guestUser{name: fields[0], home: fields[5], uid: uid, gid: gid, shell: fields[6]}, nil
	}
	return guestUser{}, fmt.Errorf("user %s not found", identifier)
}
