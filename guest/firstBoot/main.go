package main

import (
	"fmt"
	"os"

	tea "github.com/charmbracelet/bubbletea"
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
