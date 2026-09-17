package main

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

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
	green  = lipgloss.NewStyle().Foreground(lipgloss.Color("42"))
	red    = lipgloss.NewStyle().Foreground(lipgloss.Color("196"))
	yellow = lipgloss.NewStyle().Foreground(lipgloss.Color("220"))
	title  = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("205"))
)

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
