package main

import (
	"bytes"
	"image"
	"image/color"
	"image/jpeg"
	"testing"
)

const query = `: eDP-1:
	Z: 0
	namespace: awww-daemon
	image: "/home/gabriel/.config/Documents/Wallpaper/12-Monterey-Dark.jpg"
	clear-color: 0x00000000
	scale: 2
`

func TestWallpaperFrom(t *testing.T) {
	want := "/home/gabriel/.config/Documents/Wallpaper/12-Monterey-Dark.jpg"
	if got := wallpaperFrom(query); got != want {
		t.Fatalf("wallpaper from query = %q, want %q", got, want)
	}
	if got := wallpaperFrom(": eDP-1:\n\timage: none\n"); got != "none" {
		t.Fatalf("wallpaper from query without image = %q", got)
	}
}

func TestToneOf(t *testing.T) {
	picture := image.NewRGBA(image.Rect(0, 0, 64, 64))
	for y := 0; y < 64; y++ {
		for x := 0; x < 64; x++ {
			picture.Set(x, y, color.RGBA{0x20, 0x30, 0x48, 0xff})
		}
	}
	var encoded bytes.Buffer
	if err := jpeg.Encode(&encoded, picture, nil); err != nil {
		t.Fatalf("encode: %v", err)
	}
	tone, ok := toneOf(&encoded)
	if !ok {
		t.Fatal("tone of a jpeg not found")
	}
	if tone != "0x203048" {
		t.Fatalf("tone = %s, want 0x203048", tone)
	}
	if _, ok := toneOf(bytes.NewReader([]byte("not an image"))); ok {
		t.Fatal("tone of junk reported found")
	}
}
