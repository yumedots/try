package main

import (
	"bytes"
	"crypto/md5"
	"fmt"
	"io"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
	"sync"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

type guestUser struct {
	name  string
	home  string
	uid   int
	gid   int
	shell string
}

type runtimeState struct {
	user     guestUser
	config   string
	packages string
	sources  []string
}

type action func() error

type stepStatus uint8

const (
	pending stepStatus = iota
	running
	succeeded
	failed
)

type step struct {
	name   string
	status stepStatus
}

type stepResult struct {
	index int
	err   error
}

type model struct {
	steps   []step
	actions []action
	current int
	err     error
	width   int
}

var (
	logFile *os.File
	logMu   sync.Mutex
	green   = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	red     = lipgloss.NewStyle().Foreground(lipgloss.Color("196"))
	yellow  = lipgloss.NewStyle().Foreground(lipgloss.Color("220"))
	title   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("205"))
)

func main() {
	openLog()
	defer closeLog()

	state := &runtimeState{}
	steps, actions := provision(state)
	if !interactive() {
		for index, action := range actions {
			logf("step %d: %s", index+1, steps[index].name)
			if err := action(); err != nil {
				logf("step failed: %v", err)
				fmt.Fprintf(os.Stderr, "provisioning failed: %v\n", err)
				os.Exit(1)
			}
		}
		return
	}
	m := model{steps: steps, actions: actions}
	final, err := tea.NewProgram(m, tea.WithAltScreen()).Run()
	if err != nil {
		logf("tui failed: %v", err)
		os.Exit(1)
	}
	if finalModel, ok := final.(model); ok && finalModel.err != nil {
		os.Exit(1)
	}
}

func interactive() bool {
	info, err := os.Stdin.Stat()
	return err == nil && info.Mode()&os.ModeCharDevice != 0
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
			return configureLogin(state.user.uid)
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
			if _, err := command("systemctl", "enable", "ly@tty1.service"); err != nil {
				return fmt.Errorf("enable ly: %w", err)
			}
			if _, err := command("systemctl", "restart", "ly@tty1.service"); err != nil {
				return fmt.Errorf("start ly: %w", err)
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

func (m model) Init() tea.Cmd {
	if len(m.steps) > 0 {
		m.steps[0].status = running
	}
	return m.next()
}

func (m model) next() tea.Cmd {
	if m.current >= len(m.actions) {
		return tea.Quit
	}
	return func() tea.Msg {
		logf("step %d: %s", m.current+1, m.steps[m.current].name)
		err := m.actions[m.current]()
		return stepResult{index: m.current, err: err}
	}
}

func (m model) Update(message tea.Msg) (tea.Model, tea.Cmd) {
	switch message := message.(type) {
	case tea.WindowSizeMsg:
		m.width = message.Width
	case stepResult:
		if message.err != nil {
			m.steps[message.index].status = failed
			m.err = message.err
			logf("step failed: %v", message.err)
			return m, tea.Quit
		}
		m.steps[message.index].status = succeeded
		m.current++
		if m.current == len(m.steps) {
			return m, tea.Quit
		}
		m.steps[m.current].status = running
		return m, m.next()
	}
	if m.current < len(m.steps) {
		m.steps[m.current].status = running
	}
	return m, nil
}

func (m model) View() string {
	width := m.width
	if width < 40 {
		width = 40
	}
	var b strings.Builder
	b.WriteString(title.Render("try guest provisioning"))
	b.WriteString("\n\n")
	for _, step := range m.steps {
		icon := "○"
		lineStyle := lipgloss.NewStyle()
		switch step.status {
		case running:
			icon = "◌"
			lineStyle = yellow
		case succeeded:
			icon = "✓"
			lineStyle = green
		case failed:
			icon = "×"
			lineStyle = red
		}
		b.WriteString(lineStyle.Render(icon + " " + step.name))
		b.WriteString("\n")
	}
	if m.err != nil {
		b.WriteString("\n")
		b.WriteString(red.Render("provisioning failed"))
		b.WriteString("\n")
		b.WriteString(fit(m.err.Error(), width-2))
		b.WriteString("\n\nlog: /var/log/try-firstBoot.log")
	} else if m.current == len(m.steps) {
		b.WriteString("\n")
		b.WriteString(green.Render("provisioning complete"))
	}
	return b.String()
}

func fit(value string, width int) string {
	if width < 2 || len([]rune(value)) <= width {
		return value
	}
	runes := []rune(value)
	return string(runes[:width-1]) + "…"
}

func openLog() {
	file, err := os.OpenFile("/var/log/try-firstBoot.log", os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		file, _ = os.OpenFile("/run/try-firstBoot.log", os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	}
	logFile = file
	logf("started")
}

func closeLog() {
	if logFile != nil {
		logFile.Close()
	}
}

func logf(format string, args ...any) {
	line := fmt.Sprintf("%s %s\n", time.Now().Format(time.RFC3339), fmt.Sprintf(format, args...))
	logMu.Lock()
	defer logMu.Unlock()
	if logFile != nil {
		_, _ = logFile.WriteString(line)
	}
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

func configureLogin(uid int) error {
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
	if err := os.MkdirAll("/etc/ly", 0755); err != nil {
		return fmt.Errorf("create ly config directory: %w", err)
	}
	userDefs := "/etc/ly/try-login.defs"
	if err := os.WriteFile(userDefs, []byte(fmt.Sprintf("UID_MIN %d\nUID_MAX %d\n", uid, uid)), 0644); err != nil {
		return fmt.Errorf("write ly login defs: %w", err)
	}
	configPath := "/etc/ly/config.ini"
	config, err := os.ReadFile(configPath)
	if err != nil {
		return fmt.Errorf("read ly config: %w", err)
	}
	config = setLyConfig(config, "login_defs_path", userDefs)
	config = setLyConfig(config, "type_username", "false")
	if err := os.WriteFile(configPath, config, 0644); err != nil {
		return fmt.Errorf("write ly config: %w", err)
	}
	return nil
}

func setLyConfig(data []byte, key, value string) []byte {
	pattern := regexp.MustCompile(`(?m)^\s*` + regexp.QuoteMeta(key) + `\s*=.*$`)
	line := []byte(key + " = " + value)
	if pattern.Match(data) {
		return pattern.ReplaceAll(data, line)
	}
	return append(data, append([]byte("\n"), append(line, '\n')...)...)
}
