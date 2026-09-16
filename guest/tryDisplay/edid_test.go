package main

import (
	"fmt"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const (
	blankH = 160
	frontH = 48
	syncH  = 32
	blankV = 40
	frontV = 8
	syncV  = 8
)

func windowMM(pixels int, factor float64) int {
	return int(math.Round(float64(pixels)/factor*2.54/logicalDPI)) * 10
}

func describe(descriptor []byte, width, height, widthMM, heightMM int) {
	descriptor[2] = byte(width & 0xff)
	descriptor[3] = byte(blankH & 0xff)
	descriptor[4] = byte((width>>8)<<4) | byte((blankH>>8)&0x0f)
	descriptor[5] = byte(height & 0xff)
	descriptor[6] = byte(blankV & 0xff)
	descriptor[7] = byte((height>>8)<<4) | byte((blankV>>8)&0x0f)
	descriptor[8] = byte(frontH & 0xff)
	descriptor[9] = byte(syncH & 0xff)
	descriptor[10] = byte((frontV&0x0f)<<4) | byte(syncV&0x0f)
	descriptor[11] = byte((frontH>>8)&0x03)<<6 | byte((syncH>>8)&0x03)<<4 |
		byte((frontV>>8)&0x03)<<2 | byte((syncV>>8)&0x03)
	descriptor[12] = byte(widthMM & 0xff)
	descriptor[13] = byte(heightMM & 0xff)
	descriptor[14] = byte((widthMM>>8)<<4) | byte((heightMM>>8)&0x0f)
	descriptor[17] = 0x18
}

func seal(block []byte) {
	block[len(block)-1] = 0
	block[len(block)-1] = byte((256 - sum(block[:len(block)-1])%256) % 256)
}

func sum(bytes []byte) int {
	total := 0
	for _, value := range bytes {
		total += int(value)
	}
	return total
}

func baseEDID(width, height int, factor float64) []byte {
	edid := make([]byte, 128)
	copy(edid, []byte{0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00})
	edid[18], edid[19] = 1, 4
	edid[54], edid[55] = 0x30, 0x2a
	widthMM, heightMM := windowMM(width, factor), windowMM(height, factor)
	edid[21], edid[22] = byte(widthMM/10), byte(heightMM/10)
	describe(edid[54:72], width, height, widthMM, heightMM)
	seal(edid)
	return edid
}

func writeEDID(t *testing.T, edid []byte) string {
	t.Helper()
	connector := filepath.Join(t.TempDir(), "card0-Virtual-1")
	if err := os.MkdirAll(connector, 0o755); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(connector, "edid"), edid, 0o644); err != nil {
		t.Fatal(err)
	}
	return connector
}

func TestWindowRuleCarriesTheModeAndTheWindowScale(t *testing.T) {
	for _, test := range []struct {
		points int
		factor float64
		scale  string
	}{
		{1512, 2.0, "2"},
		{1512, 1.0, "1"},
		{1000, 1.5, "1.5"},
	} {
		width := int(float64(test.points) * test.factor)
		connector := writeEDID(t, baseEDID(width, width/2, test.factor))
		rule, found, ok := windowRule(connector)
		if !ok {
			t.Fatalf("%d pt at %gx: no rule", test.points, test.factor)
		}
		mode, ok := found.timing.modeline()
		if !ok {
			t.Fatalf("%d pt at %gx: no modeline", test.points, test.factor)
		}
		want := fmt.Sprintf("hl.monitor({ output = \"Virtual-1\", mode = %q, scale = %q })", mode, test.scale)
		if rule != want {
			t.Fatalf("rule = %q, want %q", rule, want)
		}
		if found.timing.width != width {
			t.Fatalf("width = %d, want %d", found.timing.width, width)
		}
	}
}

func TestScaleLeavesWholeLogicalPixels(t *testing.T) {
	for _, size := range [][2]int{{3024, 1964}, {1512, 982}, {1000, 750}, {1512, 1514}, {1390, 1440}} {
		width, height := size[0], size[1]
		for _, requested := range []float64{1.0, 1.5, 2.0} {
			scale := cleanScale(width, height, requested)
			for _, edge := range []int{width, height} {
				if math.Mod(float64(edge)/scale, 1) > 0.0001 {
					t.Fatalf("%dx%d at %v: %d is not a whole number of logical pixels", width, height, requested, edge)
				}
			}
		}
	}
}

func TestDisplayIDTimingWinsOverTheBaseBlock(t *testing.T) {
	edid := append(baseEDID(1920, 1080, 2.0), make([]byte, 128)...)
	edid[126] = 1
	seal(edid[:128])
	extension := edid[128:]
	extension[0], extension[1], extension[2] = displayIDTag, 0x13, 23
	extension[5], extension[7] = displayIDTimingTag, timingBlock
	entry := extension[8 : 8+timingBlock]
	entry[0], entry[1], entry[2] = 0xef, 0x55, 0x00
	entry[3] = 0x80
	entry[4], entry[5] = byte((3024-1)&0xff), byte((3024-1)>>8)
	entry[6], entry[7] = byte((blankH-1)&0xff), byte((blankH-1)>>8)
	entry[8], entry[9] = byte((frontH-1)&0xff), 0x80
	entry[10], entry[11] = byte(syncH-1), 0
	entry[12], entry[13] = byte((1964-1)&0xff), byte((1964-1)>>8)
	entry[14], entry[15] = byte((blankV-1)&0xff), 0
	entry[16], entry[17] = byte(frontV-1), 0
	entry[18], entry[19] = byte(syncV-1), 0
	extension[28] = 0
	extension[28] = byte((256 - sum(extension[1:28])%256) % 256)
	seal(extension)

	_, found, ok := windowRule(writeEDID(t, edid))
	if !ok {
		t.Fatal("no rule from the DisplayID extension")
	}
	if found.timing.size() != "3024x1964" {
		t.Fatalf("size = %s, want the DisplayID timing", found.timing.size())
	}
}

func TestFallbackDensityLeavesTheScaleAlone(t *testing.T) {
	edid := baseEDID(3024, 1964, 2.0)
	widthMM, heightMM := qemuEDIDMM(3024, fallbackDPI), qemuEDIDMM(1964, fallbackDPI)
	edid[21], edid[22] = byte(widthMM/10), byte(heightMM/10)
	describe(edid[54:72], 3024, 1964, widthMM, heightMM)
	seal(edid)

	rule, _, ok := windowRule(writeEDID(t, edid))
	if !ok {
		t.Fatal("no rule without a host-described window")
	}
	if strings.Contains(rule, "scale =") {
		t.Fatalf("rule = %q, want the configured scale left alone", rule)
	}
}

func TestModelineRejectsTimingsTheCompositorCannotTake(t *testing.T) {
	if _, ok := (timing{}).modeline(); ok {
		t.Fatal("an empty timing is not a modeline")
	}
	if _, ok := (timing{clock: mhz, width: 10, height: 10}).modeline(); ok {
		t.Fatal("a timing under the minimum size is not a modeline")
	}
	complete := timing{clock: 220 * mhz, width: 3024, hblank: blankH, hfront: frontH, hsync: syncH,
		height: 1964, vblank: blankV, vfront: frontV, vsync: syncV, hsyncPositive: true}
	line, ok := complete.modeline()
	if !ok {
		t.Fatal("a complete timing should be a modeline")
	}
	if line != "modeline 220 3024 3072 3104 3184 1964 1972 1980 2004 +hsync -vsync" {
		t.Fatalf("modeline = %q", line)
	}
}

func TestPhysicalSizeUsesTheBaseBlock(t *testing.T) {
	widthMM, heightMM := physicalSize(baseEDID(3024, 1964, 2.0))
	if widthMM != 350 || heightMM != 230 {
		t.Fatalf("physical size = %dx%d mm, want 350x230", widthMM, heightMM)
	}
}

func TestALargerWindowLandsOnALargerScale(t *testing.T) {
	widthMM, heightMM := windowMM(3024, 2.0), windowMM(1964, 2.0)
	smaller := displayScale(2056, 1326, widthMM, heightMM)
	reference := displayScale(3024, 1964, widthMM, heightMM)
	fullscreen := displayScale(3840, 2160, widthMM, heightMM)

	if math.Abs(reference-2) > 0.05 {
		t.Fatalf("the window the host opens at scaled to %v, want 2", reference)
	}
	if smaller >= reference || reference >= fullscreen {
		t.Fatalf("scales %v, %v, %v do not grow with the window", smaller, reference, fullscreen)
	}
}

func TestTheHostRateReadsBackFromTheClock(t *testing.T) {
	for _, test := range []struct {
		width, height, hz int
	}{
		{3024, 1964, 60},
		{3024, 1964, 75},
		{1512, 982, 120},
	} {
		edid := baseEDID(test.width, test.height, 2.0)
		clock := test.hz * (test.width + blankH) * (test.height + blankV) / kilohertz
		if clock > 0xffff {
			t.Fatalf("%dx%d at %d Hz does not fit the base block", test.width, test.height, test.hz)
		}
		edid[54], edid[55] = byte(clock&0xff), byte(clock>>8)
		seal(edid)

		_, found, ok := windowRule(writeEDID(t, edid))
		if !ok {
			t.Fatalf("%d Hz: no rule", test.hz)
		}
		want := fmt.Sprintf("%dx%d@%d", test.width, test.height, test.hz)
		if rate := found.timing.rate(); rate != want {
			t.Fatalf("%s read back as %s", want, rate)
		}
	}
}
