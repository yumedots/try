package main

import "os"

const serialPort = "/dev/ttyAMA0"

func tell(message string) {
	device, err := os.OpenFile(serialPort, os.O_WRONLY, 0)
	if err != nil {
		return
	}
	defer device.Close()
	device.WriteString(message)
}
