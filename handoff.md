# Handoff: continue the try VM display work

You are picking up work on `try`: a VM launcher for macOS (and Linux) whose window shows
a Linux guest, with the window size driving the guest's display. Read `AGENTS.md`,
`README.md` and `PLANS.md` first, then this.

## The goal we are chasing

`xmake run` opens the GPUI window with the guest inside it. It must feel the same as
`xmake vm` (QEMU's own cocoa window): resizes the guest live while you drag, no visible
re-mode, low latency. Today the cocoa window does that at ~120 fps; the GPUI window does
it at **~10 fps** with ~100 ms of lag per frame.

## Repos and where things are

- `~/try` — this repo. `xmake run` builds `ui/` and opens the GPUI window (it starts QEMU
  itself with `xmake vm --bridge --gl`). `xmake vm` boots the same guest in QEMU's own
  cocoa window. `xmake vm --bridge` exports the display over D-Bus instead of a window.
  `xmake reset` rolls the disk back to the `provisioned` snapshot. `xmake clean --cache`
  drops downloads.
- `~/qemu` — the QEMU fork (`yumedots/qemu`), branch `master`, currently **clean**. Build
  with `ninja -C buildLocal qemu-system-aarch64`; install a local build into the project
  with `/tmp/localqemu.sh` (rewrites dylib paths into `.cache/qemu/opt/*`, re-signs). CI
  builds a bottle per commit on GitHub releases; `xmake.lua` pins one by URL + sha256
  (`shared.qemubottles`, currently the commit `7d46e13418…`). `xmake fetch` restores the
  pinned bottle over a local build.
- `~/gpui` — clone of the GPUI fork (`yumedots/gpui`, branch `master`, head `f2571b5`).
  `ui/Cargo.toml` depends on it by rev for `gpui`, `gpui_platform` and (test-only)
  `gpui_apple`. There is **no `vendor/` any more**: `f2571b5` is our own commit
  (`feat: draw a caller owned bgra surface` — Metal renderer + shader for a caller-owned
  BGRA `CVPixelBuffer`), on top of sonorahq/gpui's layer filters (`Styled::blur`,
  `Styled::backdrop_blur`, `Styled::layer_scale`, …). If you need to change gpui, commit in
  `~/gpui`, push, and bump the rev in `ui/Cargo.toml`.
- Guest: generic Arch Linux ARM aarch64 (mirror picked at build time), SDDM → Hyprland,
  dotfiles from the `dotfiles` submodule. ssh `alarm@127.0.0.1:2222` (password `alarm`),
  forwarded by `-netdev user,hostfwd=…` — available in the bridge run too.

## What already works (do not regress)

- **cocoa window resize** (`xmake vm`): the window is never conformed to a mode the guest
  just adopted, live-resize sizes are sent while you drag, and `ui/console.c` notifies the
  guest on a 100 ms rate limit instead of a 1 s settle timer. The window no longer walks
  itself down; the guest follows during the drag.
- **Guest side** (`guest/tryDisplay`): follows the window (mode + density → desktop scale,
  quantised to 1.0/1.5/2.0), reacts to the kernel's DRM change event instead of polling,
  and sets `misc:background_color` to the wallpaper's own tone so awww's reload does not
  flash black (`background.go`, `background_test.go`).
- **Design rule**: the mode we ask for is the window in pixels aligned to the scale divisor;
  the density in the EDID comes from the window's size in whole cm at ~110 dpi
  (`LOGICAL_DPI` in `ui/src/main.rs`, `cocoa_mode_*` in `ui/cocoa.m`).

## The actual problem: the GPUI window's frame path

Per frame today: QEMU `glReadPixels` of the guest's GL texture (`dbus_gl_surface_read` in
`~/qemu/ui/dbus-listener.c`) → ~14 MB over the p2p D-Bus connection (`dbus_gl_surface_send`)
→ the app builds a gpui `RenderImage` (convert + texture upload) → draw. Three CPU hops.

Measured (debug build, 2560x1440):

| path | frame interval | guest Hyprland CPU |
|---|---|---|
| `--gl` (default) | 117 ms median (~8.6 fps) | 15.8 % |
| `TRY_NO_GL=1` | 100 ms median (~10 fps) | ~400 % (llvmpipe) |

**GPU stays on** — no-GL is not faster *and* it wrecks the guest. The cost is the transport,
not the render. QEMU's zero-copy D-Bus path (`dmabuf`) is Linux-only, so macOS always falls
back to the readback.

### The plan (agreed, not written yet)

1. **QEMU half**: let the client give QEMU a buffer — a `SetSurface(iosurface_id, stride)`
   method on `org.qemu.Display1.Console` (`ui/dbus-display1.xml` + `ui/dbus-console.c`), and
   in `dbus_gl_surface_read()` `IOSurfaceLookup` + lock that surface and `glReadPixels`
   *into* it instead of into the software `DisplaySurface`; then send a **tiny ping**
   (an `Update` with no payload, or a new Listener call) instead of the pixels.
2. **App half**: a ring of caller-owned `CVPixelBuffer`s (the GPU still holds the one being
   drawn, so locking it fails — that is why it needs ≥2), one memcpy per frame, drawn with
   `guestSurface()` / `gpui::surface()` — no `RenderImage`, no upload.
3. **Best case after that**: drop the readback too — draw the guest's scanout *into* that
   IOSurface on the GPU. ANGLE has `EGL_ANGLE_iosurface_client_buffer` (an EGL pbuffer from
   an IOSurface) but it is a draft aimed at iOS: **verify it on macOS before relying on it.**
   Also send `refresh_rate` in `SetUIInfo` so the guest paces to the panel.

An app-half-only attempt was made and **reverted**: frames copied into a 3-buffer ring and
drawn with `gpui::surface()`; it built and passed 12 tests but the window came up black
live, and the cause was not diagnosed. The receiver cannot be right without the sender —
do the QEMU half first, then bring the app half back.

## Also missing right now

**The resize blur is not in the tree.** It was working (user-confirmed: resizes, and
unblurs when it settles) and lived in the working copy that was reverted to keep the app
usable. To put it back:

- `Styled::blur(px(28.0))` on the frame while the window is being resized, off when it
  settles — a GPU layer filter from the fork, no CPU work.
- A hold driven by the *window* changing size (every change pushes it out ~500 ms,
  `BLUR_HOLD`), because holding it until a matching frame arrives never ends: through the
  bridge the guest does not always send a frame the window's size.
- `Resize { last, until }` with `changed(size, now)` / `still_up(now)` was the shape; a test
  asserted that a repeated size does not hold it and that it clears on its own.

## Loose ends and gotchas

- Uncommitted in `~/try`: `guest/tryDisplay/{background.go,background_test.go,main.go,
  edid_test.go}`, `ui/Cargo.toml` + `ui/Cargo.lock` (the fork pin), `ui/xmake.lua`
  (`xmake run` → the GPUI app), `xmake.lua` (the bottle re-pin). `~/qemu` is clean.
- Do not commit or push anything without being asked. Commit in small pieces, one-line
  messages matching the repo's style, no comments in code, camelCase file names.
- Every dependency is fetched into `.cache` and run from there, never installed system-wide.
- The app kills its own QEMU when the window closes (that is the `terminating on signal 15`
  you may see in its log — expected, the user closing the window).
- Never run two VMs on the same disk.
- Asahi ALARM is **not** usable here: it is the bare-metal Apple Silicon variant (linux-asahi
  kernel, m1n1/U-Boot chain, Apple GPU drivers) and under `-machine virt` + HVF the guest
  only sees virtio devices. Keep generic ALARM.
- Driving the guest from the host: resize the app window with AppleScript
  (`tell process "try-ui" to set size of window 1`) — **do not synthesise mouse events**,
  they fight the user's own cursor. Capture with a ScreenCaptureKit window filter (a window
  filter ignores occlusion). Renderer tests run headless through
  `gpui_apple::metal_renderer::MetalHeadlessRenderer` — that is how the caller-owned surface
  path is checked without a VM.
