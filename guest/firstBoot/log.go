package main

import (
	"fmt"
	"os"
	"sync"
	"time"
)

var (
	logFile *os.File
	logMu   sync.Mutex
)

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
