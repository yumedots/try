package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"
)

const (
	minimumWidth  = 64
	minimumHeight = 64
	maximumTotal  = 65535

	mhz              = 1000000
	mhzRounding      = 500000
	kilohertz        = 10000
	scaleSteps       = 120
	minimumScaleStep = 30
	maximumScaleStep = 480

	mmPerInch     = 25.4
	logicalDPI    = 110.0
	fallbackDPI   = 100
	fallbackScale = 0.0

	displayIDTag       = 0x70
	displayIDTimingTag = 0x03
	timingBlock        = 20
)

type timing struct {
	clock                         int
	width, hblank, hfront, hsync  int
	height, vblank, vfront, vsync int
	hsyncPositive, vsyncPositive  bool
}

func (t timing) size() string {
	return fmt.Sprintf("%dx%d", t.width, t.height)
}

func (t timing) rate() string {
	return fmt.Sprintf("%s@%d", t.size(), t.refresh())
}

func (t timing) refresh() int {
	total := (t.width + t.hblank) * (t.height + t.vblank)
	if t.clock <= 0 || total <= 0 {
		return 0
	}
	return (t.clock + total/2) / total
}

func (t timing) modeline() (string, bool) {
	if t.clock <= 0 || t.width < minimumWidth || t.height < minimumHeight ||
		t.hfront <= 0 || t.hsync <= 0 || t.vfront <= 0 || t.vsync <= 0 ||
		t.hblank < t.hfront+t.hsync || t.vblank < t.vfront+t.vsync ||
		t.width+t.hblank > maximumTotal || t.height+t.vblank > maximumTotal {
		return "", false
	}
	whole := (t.clock + mhzRounding) / mhz
	if whole < 1 {
		whole = 1
	}
	return fmt.Sprintf("modeline %d %d %d %d %d %d %d %d %d %s %s",
		whole,
		t.width, t.width+t.hfront, t.width+t.hfront+t.hsync, t.width+t.hblank,
		t.height, t.height+t.vfront, t.height+t.vfront+t.vsync, t.height+t.vblank,
		polarity("hsync", t.hsyncPositive), polarity("vsync", t.vsyncPositive)), true
}

func polarity(name string, positive bool) string {
	if positive {
		return "+" + name
	}
	return "-" + name
}

type display struct {
	timing            timing
	widthMM, heightMM int
}

func preferredDisplay(edid []byte) (display, bool) {
	if len(edid) < 128 || !header(edid) || !wholeBlock(edid[:128]) {
		return display{}, false
	}
	if found, ok := displayIDDisplay(edid); ok {
		return found, true
	}
	return baseDisplay(edid)
}

func displayIDDisplay(edid []byte) (display, bool) {
	available := len(edid)/128 - 1
	count := int(edid[126])
	if count > available {
		count = available
	}
	var first display
	found := false
	for index := 1; index <= count; index++ {
		block := edid[index*128 : index*128+128]
		if block[0] != displayIDTag || block[1]>>4 != 1 || !wholeBlock(block) {
			continue
		}
		end := 5 + int(block[2])
		if end > 126 || !wholeBlock(block[1:end+1]) {
			continue
		}
		widthMM, heightMM := physicalSize(edid)
		for position := 5; position+3 <= end; {
			tag := block[position]
			length := int(block[position+2])
			start := position + 3
			stop := start + length
			if stop > end {
				break
			}
			if tag == displayIDTimingTag && length > 0 && length%timingBlock == 0 {
				for offset := start; offset < stop; offset += timingBlock {
					entry := block[offset : offset+timingBlock]
					candidate := display{timing: displayIDTiming(entry), widthMM: widthMM, heightMM: heightMM}
					if _, ok := candidate.timing.modeline(); !ok {
						continue
					}
					if entry[3]&0x80 != 0 {
						return candidate, true
					}
					if !found {
						first, found = candidate, true
					}
				}
			}
			position = stop
		}
	}
	return first, found
}

func displayIDTiming(entry []byte) timing {
	return timing{
		clock:         (little24(entry) + 1) * kilohertz,
		width:         int(little16(entry[4:])) + 1,
		hblank:        int(little16(entry[6:])) + 1,
		hfront:        int(little16(entry[8:])&0x7fff) + 1,
		hsyncPositive: little16(entry[8:])&0x8000 != 0,
		hsync:         int(little16(entry[10:])) + 1,
		height:        int(little16(entry[12:])) + 1,
		vblank:        int(little16(entry[14:])) + 1,
		vfront:        int(little16(entry[16:])&0x7fff) + 1,
		vsyncPositive: little16(entry[16:])&0x8000 != 0,
		vsync:         int(little16(entry[18:])) + 1,
	}
}

func baseDisplay(edid []byte) (display, bool) {
	for offset := 54; offset+18 <= 126; offset += 18 {
		descriptor := edid[offset : offset+18]
		if little16(descriptor) == 0 || descriptor[17]&0x80 != 0 || descriptor[17]&0x61 != 0 ||
			descriptor[17]&0x18 != 0x18 {
			continue
		}
		candidate := display{
			timing: timing{
				clock:         int(little16(descriptor)) * kilohertz,
				width:         int(descriptor[2]) | int(descriptor[4]&0xf0)<<4,
				hblank:        int(descriptor[3]) | int(descriptor[4]&0x0f)<<8,
				hfront:        int(descriptor[8]) | int(descriptor[11]&0xc0)<<2,
				hsync:         int(descriptor[9]) | int(descriptor[11]&0x30)<<4,
				height:        int(descriptor[5]) | int(descriptor[7]&0xf0)<<4,
				vblank:        int(descriptor[6]) | int(descriptor[7]&0x0f)<<8,
				vfront:        int(descriptor[10]>>4) | int(descriptor[11]&0x0c)<<2,
				vsync:         int(descriptor[10]&0x0f) | int(descriptor[11]&0x03)<<4,
				hsyncPositive: descriptor[17]&0x02 != 0,
				vsyncPositive: descriptor[17]&0x04 != 0,
			},
			widthMM:  int(descriptor[12]) | int(descriptor[14]&0xf0)<<4,
			heightMM: int(descriptor[13]) | int(descriptor[14]&0x0f)<<8,
		}
		if _, ok := candidate.timing.modeline(); !ok {
			continue
		}
		if candidate.widthMM == 0 || candidate.heightMM == 0 {
			candidate.widthMM, candidate.heightMM = physicalSize(edid)
		}
		return candidate, true
	}
	return display{}, false
}

func physicalSize(edid []byte) (int, int) {
	return int(edid[21]) * 10, int(edid[22]) * 10
}

func qemuEDIDMM(resolution, dpi int) int {
	return resolution * 254 / 10 / dpi
}

func displayScale(width, height, widthMM, heightMM int) float64 {
	if widthMM <= 0 || heightMM <= 0 || widthMM == qemuEDIDMM(width, fallbackDPI) {
		return fallbackScale
	}
	diagonal := math.Hypot(float64(widthMM), float64(heightMM)) / mmPerInch
	if diagonal <= 0 {
		return fallbackScale
	}
	return cleanScale(width, height, math.Hypot(float64(width), float64(height))/diagonal/logicalDPI)
}

func cleanScale(width, height int, requested float64) float64 {
	target := int(math.Round(requested * scaleSteps))
	best := 0
	for candidate := minimumScaleStep; candidate <= maximumScaleStep; candidate++ {
		if (width*scaleSteps)%candidate != 0 || (height*scaleSteps)%candidate != 0 {
			continue
		}
		if best == 0 || distance(candidate, target) < distance(best, target) ||
			(distance(candidate, target) == distance(best, target) && candidate > best) {
			best = candidate
		}
	}
	if best == 0 {
		return requested
	}
	return float64(best) / scaleSteps
}

func distance(left, right int) int {
	if left > right {
		return left - right
	}
	return right - left
}

func displayRule(output string, found display, scale float64) (string, bool) {
	mode, ok := found.timing.modeline()
	if !ok {
		return "", false
	}
	fields := []string{
		fmt.Sprintf("output = %q", output),
		fmt.Sprintf("mode = %q", mode),
	}
	if scale > fallbackScale {
		fields = append(fields, fmt.Sprintf("scale = %q", strconv.FormatFloat(scale, 'f', -1, 64)))
	}
	return "hl.monitor({ " + strings.Join(fields, ", ") + " })", true
}

func header(edid []byte) bool {
	expected := []byte{0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00}
	for index, value := range expected {
		if edid[index] != value {
			return false
		}
	}
	return true
}

func wholeBlock(block []byte) bool {
	total := 0
	for _, value := range block {
		total += int(value)
	}
	return total%256 == 0
}

func little16(bytes []byte) int {
	return int(bytes[0]) | int(bytes[1])<<8
}

func little24(bytes []byte) int {
	return int(bytes[0]) | int(bytes[1])<<8 | int(bytes[2])<<16
}
