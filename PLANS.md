# try — plan

Goal: run this dotfiles desktop in a VM on Mac, like try-omarchy. No Docker, no QEMU build, no sign.

## Layout

```
try/
  dotfiles/      submodule -> dotfiles repo, at root
  guest/firstBoot/main.go guest/packages.txt
  qemu/          arch matrix notes only
  ui/            GPUI launcher, locked to 1 commit hash
  xmake.lua      all pinned URLs + all tasks
  build/         cache, gitignored
```

## Rules

- Same dotfiles all machines. `envs.lua` auto-detects nvidia (`lspci`), sets vars only then.
- xmake fetches all: prebuilt QEMU per os/arch + stock Arch ISO per arch, pinned by hash. Builds only GPUI app.
- No compile of QEMU/guest. No signing, opensource.
- ARM now (Mac testable). x86_64 later, same script. Mac now, Linux later. No Windows.

## Flow

1. `xmake fetch`: QEMU bottles + Arch tarball -> `.cache/`, hash-pinned.
2. First boot: systemd once-unit runs the Go program from `guest/firstBoot/main.go`: install `guest/packages.txt`, link baked `dotfiles/*` -> `~/.config/*`, then disables itself.
3. Persistent disk: pay once, reuse every boot.
4. Dotfiles: copied into the guest image at build time. The guest home stays on the persistent disk.
5. QEMU: HVF accel, virtio-gpu virgl. Homebrew QEMU now, prebuilt per platform later.

## Resizable display architecture

The GPUI launcher will own the VM window and act as the display frontend instead of relying on QEMU Cocoa scaling.

1. Keep QEMU, ISO, and guest provisioning unchanged while the guest boots reliably.
2. Start QEMU with virtio-GPU and the D-Bus display backend. `xres`/`yres` are the EDID's preferred mode and its upper bound, so the guest boots into a known size instead of the 1024x768 default.
3. Add a GPUI display bridge that receives the guest framebuffer and forwards the window's current size to QEMU with `SetUIInfo`, whenever it differs from the last request. The window is the source of truth; tracking resize *events* instead leaves the guest at its boot mode for the size the window opened at, which is a letterboxed guest screen inside a smaller window.
4. Let virtio-GPU notify the Linux guest, then let Wayland/Hyprland relayout at the new output size.
5. Keep a fixed-size fallback for hosts or QEMU builds without the display bridge.

The window image covers the window (`ObjectFit::Cover` at the window's own size, pointer mapped through the same scale and the same even crop): a guest that is the size of the window lands one guest pixel per device pixel with nothing cropped, and one that is a resize behind fills the window with its middle shown rather than growing black bars or being squeezed to the window's shape. A resize that never arrives is then reported in the log rather than shown as a crop: on a frame whose shape differs from the one the window asked for, the launcher prints `guest sent 2304x1440 for a 2560x1440 window`, and the guest's own service prints what the compositor or the framebuffer console ended up with (`compositor 2304x1440 scale 2`, or `compositor 2304x1440 does not fill 2560x1440`).

The guest applies the resize twice over, on purpose: once when the window's rule changes, and again when the kernel mode it asked for actually lands, because the compositor and the greeter are told before the mode is in and would otherwise keep drawing for the size that just went away (their picture in a corner of the new screen, or the greeter's box off centre). Detecting that is a comparison of the connector's current mode against the size the rule asked for, so it cannot loop: the second apply sets the same mode again and nothing changes after it.

The framebuffer console - the greeter, and everything on screen before a session starts - does not follow a mode change at all: the kernel keeps the framebuffer it built at driver probe, so a window that shrinks leaves the greeter drawing for the old, wider screen and its box lands in the corner of the crop the new mode shows (measured: box border columns 739..1565 in a 1600-wide frame, a centre of 1152 against the frame's 800). The guest asks the kernel for the size itself with `FBIOPUT_VSCREENINFO` on `/dev/fb0` - what `fbset` does - and the console re-lays out to it (`fitConsole`; measured 1600x1000 gives a 200x62 grid, 2400x1400 gives 300x87). What that ioctl cannot do is grow past the framebuffer, so the launcher opens the guest at the size the device advertises with `xres`/`yres` and only takes the window's own size once the guest's display service reports ready (`opening_size`): every resize after that is a size inside the framebuffer. Measured with a booted guest: 2560x1440 at boot, 1400x800 and then 2400x1400 as the window asked, the console equal to the window each time and the greeter's box borders symmetric about the frame centre within two characters.

The guest is told the size twice over, both through the EDID the launcher writes with `SetUIInfo`: the pixel size asks for that many guest pixels, and the physical-size fields carry the window in points described at 110 dpi. The guest's display service reads the density back (`pixels / inches / 110`) and applies the scale that matches it, so 1x, 1.5x and 2x hosts land on Hyprland scales 1, 1.5 and 2, one guest logical pixel is one host point, and the desktop comes out the size of the window instead of whatever the guest's own monitor configuration asks for. A size QEMU derived itself at its own 100 dpi fallback means no host described a window, and then the configured scale is left alone.

A resize is applied to the compositor as a complete modeline taken from the EDID, not as a reload and not as "preferred": Aquamarine does not refresh its mode cache when an existing connector's EDID changes, so a rule that says "preferred" resolves to the size the screen used to have and the desktop keeps painting the old dimensions inside the new framebuffer. The modeline is built from the timing QEMU publishes for the window's size - checking the DisplayID extension first, because that is where QEMU puts anything past the base block's 12-bit fields - and the rule is applied to the output by name, which replaces the entry the guest's own monitor configuration made for it (`hl.monitor` inherits the existing rule and overrides only the fields given, and Hyprland matches rules last-first). Because the mode is explicit, the compositor is not limited to the modes the EDID happens to list.

The size forwarded to the guest is the window size in *device* pixels, not in points: the image is one image pixel per point of viewport, so a guest asked for the point size is drawn upscaled by the display scale factor and comes out huge and soft, and the guest's own login screen reflows to a resolution half the size it should be. `display_pixels` does that conversion, and the window opens at 1280x720 points so a 2x display asks for the guest's native 2560x1440 and is pixel exact from the first frame.

The request has to stay inside the size the QEMU device advertises with `xres`/`yres`. Past it the guest drops the new EDID preferred mode (`virtio_gpu_conn_mode_valid` wants it within 16px of the display info it saw last) and silently settles on the largest mode left in the list, which is 1920x1440: the window then shows a guest screen of a different shape and nothing looks resized. Measured against a real guest with the device left at 2560x1440, 2000x1200 and 2560x1300 are adopted exactly while 2800x1000, 2000x1500 and 1512x1566 fall back; with the device at 6016x3384 all four are adopted exactly. So the launcher passes the host screen in device pixels as `--width`/`--height`: that is both the guest's boot mode and its ceiling, and any window that fits the screen follows exactly. `clamp_to_display` still scales a request into that box, which only matters for a window bigger than the screen.

The frame is drawn at an explicit size rather than left to the image's own fit: GPUI draws an image with one image pixel per *point*, so on a 2x display the guest's screen would be drawn at twice its size and clipped - a 1438x1440 guest in a 695x720 point window shows only its top-left quarter, with the guest's login box cut off at the bottom right corner. `fitted_size` scales the guest into the window in points, which is one guest pixel per device pixel when the two match, and the scale it uses is the same one `guest_position` maps clicks through, so the picture and the pointer agree.

The guest agent only nudges and reports: it writes `detect` to the connector so the kernel re-reads the EDID, and prints `connector`/`mode`/`session` to the serial log. It must never exit, because the connector may not exist yet when it starts. Resizing itself is QEMU's EDID plus the guest kernel, so it keeps working when the agent is not running.

A resize is checkable without a screen. `TRY_GRAB=<path>` runs the display bridge with no window and writes a frame to a file (`.ppm` writes raw pixels, so one frame can be compared against another without a decoder), and `TRY_GRAB_SIZE=<width>x<height>` - in the points a window of that size is - asks for that size first, waits for a frame that arrives at it, and fails the run if the guest never gets there; `TRY_GRAB_WAIT=<seconds>` writes the newest frame after that many seconds instead. Measured against a booted guest with the device advertising 2560x1440: 1000x600 points (2000x1200 device pixels), 1600x800 (2560x1280, the clamp of a 3200x1600 request) and 640x400 (1280x800) are each adopted exactly, within seconds of the request. `QEMU exited` in the frame now carries its exit status or signal, which is how a guest panic and a QEMU crash are told apart.

The work order is: guest provisioning, QEMU D-Bus display export, GPUI framebuffer window, resize events, guest output verification, then fullscreen and fallback testing.

## GPU by host

`virtio-gpu-gl-pci` on both hosts, only accel and display backend differ.

- macOS arm64 Tahoe 26+: `qemu-virgl` bottle (VirGL/ANGLE, `hvf`, `cocoa,gl=es`), pin + sha256 in `xmake.lua`, unpacked to `.cache/qemu`, missing Homebrew deps pulled from their own bottles, no brew install and no build. Resize follows the QEMU window natively.
- Linux: stock `qemu-system-*` from PATH, `kvm`, `gtk,gl=on`. Resize follows the window natively (upstream virtio-gpu `SetUIInfo`/EDID).
- The GPUI bridge keeps `virtio-gpu-pci` and `-display dbus,gl=off`: QEMU only exports `ScanoutDMABUF` on `CONFIG_GBM` builds (Linux), and the launcher renders shared-memory scanouts, so gl and the in-process window are mutually exclusive.
- `xmake vm --no-gl` falls back to a PATH QEMU without virtio-gpu-gl.

## Guest state and features

The guest disk lives in the state dir (`state/` in the project, overridable with `TRY_STATE_DIR`), not in `build/`, so it survives `xmake clean`. `xmake reset` drops it and the next boot rebuilds it from cache.

`settings` in the same dir holds `cores`, `mem` and `audio`. The GPUI bar writes it, `xmake vm` reads it as the default for `-smp`/`-m`/`-audiodev`, so there is one source of truth for resources. Changing a resource restarts QEMU; `reset guest` drops the disk, rebuilds it, and boots.

- Audio: `-audiodev coreaudio` on macOS, `pa` on Linux, `intel-hda` + `hda-output` in the guest, pipewire/wireplumber from `guest/packages.txt`.
- Nested KVM: `virtualization=on` is probed once per host (cached in `.cache/virtualization`) and used when the host supports it, so the guest gets `/dev/kvm`.
- The kernel is the host's, not the guest's: `xmake vm` boots the `Image` from the cached tree, so `guest/firstBoot` writes `IgnorePkg = linux-aarch64` into `/etc/pacman.conf` before installing anything. Without it a guest `pacman -Syu` upgrades the kernel in the disk and leaves no modules for the running one - the display driver never binds and the VM comes up with no GPU at all (measured: `[drm] virtio-gpu detected` on the first boot, absent on every boot after a provisioning that installed 7.2.6 under a host booting 7.1.6).

## xmake tasks

`doctor fetch guest vm run ui build package clean`. `xmake run` is the entry point: it fetches the pinned QEMU, builds the disk, builds GPUI and launches it, and GPUI boots QEMU with `--bridge` and hosts the framebuffer. `xmake vm` is the raw QEMU fallback with QEMU's own window. `is_os/is_arch` picks URLs. `depend.on_changed` skips re-fetch.

## Our own qemu

The bottle comes from a patched QEMU tree: upstream plus 28 files (`ui/cocoa.m`, `ui/egl-helpers.c`, `ui/console.c`, gtk/sdl2/spice, `ui/dbus-*.c`, `hw/display/virtio-gpu-gl.c`, `virtio-gpu-virgl.c`, `accel/hvf/hvf-all.c`, `meson.build`) — macOS GL through ANGLE, Venus/blob and a guest-stall fix, an hvf unaligned-RAM guard. Upstream has none of it: `meson_options.txt` has no `angle`, and `ui/cocoa.m` has no `CONFIG_EGL` at all.

`yumedots/qemu` holds that tree (`261e32a`, one commit) plus our commits, so the artifact is pinned by commit sha:

- `.github/workflows/buildQemu.yml` on a `macos-26` runner installs the ANGLE/libepoxy-angle/virglrenderer bottles, derives every dependency prefix (keg-only vde/libssh/ncurses/snappy/lzo/dtc included) into `--extra-cflags/--extra-ldflags`, builds `--target-list=aarch64-softmmu`, clears extended attributes and ad-hoc signs with `com.apple.security.hypervisor` (hvf needs it), then proves `libepoxy` and `libvirglrenderer` are linked and `virtio-gpu-gl-pci` is in `-device help`.
- Publishes `qemu-master.arm64_tahoe.tgz` to a release tagged `qemu-<sha>`: GitHub rejects a bare 40-hex tag. `shared.qemubottles` pins the release URL and the sha256, one entry with a `release` override, so a rebuild is a two-value bump plus `xmake fetch`.
- Only QEMU is rebuilt. The three graphics libraries still come from the existing release, which is what `shared.qemubottles` already pins.
- Our commits on it: the build workflow, then `53bbec375e` — the D-Bus listener reads a GL scanout back into a software surface on macOS and sends it as the same `Scanout`/`Update` pixels a 2D one is sent as, and `egl_init` asks ANGLE for its OpenGL backend when the display has no window (the cocoa frontend calls `qemu_egl_init_dpy_cocoa` itself, so its path is untouched).
- Verified: fetch unpacks and patches the keg, `hvf` entitlement survives the patch step, deps resolve as before, and the guest boots on it (`tryDisplay: mode 3024x1964`). `-display cocoa,gl=es` cannot be checked on a headless runner, so the build proves GL by linkage instead.

## GPU display (scoped, not built)

Today: `-display dbus,gl=off`, so QEMU sends CPU scanout updates and GPUI draws them as a `RenderImage` (CPU BGRA frames). Two halves are missing before the guest's GL can reach the window:

- **QEMU:** its D-Bus listener delivers a GL scanout only under `CONFIG_GBM` (Linux dmabuf) or `WIN32` (D3D11 shared texture). On macOS that branch is empty. The texture borrowing in the patched tree is in-process only — it feeds the cocoa frontend, not a client across a socket. A macOS delivery needs a handle that survives a process boundary: an `IOSurface` id (`IOSurfaceLookup` is cross-process), published next to the scanout. Unknown until tried: whether the GL scanout is IOSurface-backed (ANGLE/Metal) at all.
- **try (done):** GPUI has no seam for a texture it does not own — `RenderImage` is `id` + `scale_factor` + CPU `Frame`s, `ImageSource` is `Resource`/`Render`/`Image`/`Custom`, and the macOS renderer uploads those frames into its own textures. What it does have is a macOS-only `surface()` element (`elements/surface.rs` + `Window::paint_surface`) that draws a caller-owned `CVPixelBuffer` with `CVMetalTextureCache` — the texture is made from the buffer's `IOSurface`, no copy — but the renderer asserted `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange` (biplanar video), so a BGRA screen buffer could not use it.

  So: `vendor/gpui_apple` is the Apple renderer crate patched to take that second format. It is the *only* vendored crate; `gpui`, `gpui_macos`, `gpui_platform` stay the pinned zed rev, and `ui/Cargo.toml` redirects just it with `[patch."https://github.com/zed-industries/zed.git"]`. Layout: the four upstream sources verbatim, plus a checked-in `src/scene.h` (the cbindgen header for the pinned gpui, so the vendored build script needs no gpui checkout and no cbindgen) and our diff, which is the `surfaces_bgra_pipeline_state` pipeline plus one branch in `draw_surfaces` and a `surface_bgra_fragment` in `shaders.metal`. Regenerate `scene.h` from the pinned rev if the zed rev moves.

  try side: `ui/src/guestSurface.rs` — `GuestSurface` owns an IOSurface-backed BGRA `CVPixelBuffer` (`io_surface_id()` is the handle a QEMU delivery would hand back) and `guestSurface()` is the element. The frame arrives through `TRY_SURFACE=1` as a host-drawn pattern until the QEMU half exists; the check is `the_window_draws_the_callers_buffer`, which renders a scene through gpui's Metal headless renderer and asserts the read-back pixels come from the caller's buffer, then writes one pixel into the same buffer and asserts the next frame shows it — the second half is what a copy could not pass, and the whole test fails outright with the BGRA branch disabled.

  The other route, tried in the fork first because it needs no IOSurface and no client change: read the borrowed texture back with `glReadPixels` and send it through the ordinary `Scanout`/`Update` calls. It works as far as the D-Bus protocol goes — frames arrive — but the platform dies underneath it. On macOS libepoxy hands out the **system** OpenGL library's entry points, and those functions have no context of their own while ANGLE owns the rendering, so the first call segfaults. Measured in both places: our readback (`egl_fb_setup_for_tex` → `glGenFramebuffers`) and, before it, upstream's own `surface_gl_destroy_texture` → `glDeleteTextures`, reached when the guest resets the GPU during boot. Taking the entry points from the EGL display instead of libepoxy got past the first crash, and asking ANGLE for its OpenGL backend instead of Metal did not change which library the calls land in.

  Two things were fixed in the fork to get past both:

  1. **A virgl scanout of a 2D resource was rejected outright.** `virgl_cmd_set_scanout` called `virtio_gpu_check_scanout_bounds(..., 0, 0, ...)` — a zero-sized box, so *every* rect failed it. Upstream passes the resource's own width and height there. Measured on the guest: `[drm:virtio_gpu] *ERROR* response 0x1205 (command 0x103)` twice at every boot, `fb0` registered but no scanout set up, and QEMU's window saying `Display output not active` with the host console showing instead; after the fix the guest's mode is set (2560x1440) with no error, and the same boot on `-display cocoa,gl=es` has an output. That failure is also what reset the GPU (the reset path is where the GL crash was reached from), so it is what made the GL display die at every boot.

  2. **QEMU's GL calls went to the wrong library.** libepoxy resolves GL through the platform's GL — Apple's, which owns no context in our process — while the context belongs to ANGLE. Measured with `dladdr` on each pointer: `glDeleteTextures` and friends resolved to libepoxy (which then dlopened `libGL.dylib`), and the crash moved from `glDeleteTextures` to `glGetString` as each was fixed. `egl_init` now takes the entry points the display path uses — 55 of them, the display's calls and epoxy's own internal ones — from the EGL display, so they and the context are the same implementation. Crashes are gone.

  What is left: `-display dbus,gl=on` now boots, survives, and delivers frames (the console's 2D scanout arrives at the right size), but the GL readback comes back black with `dbus: frame buffer read back failed` (a GL error from `glReadPixels` on the borrowed texture — the texture id comes from virglrenderer's context and QEMU's own context is not shared with it, which EGL's default share group does not give us). That is the next thing to fix, and it is why the GL display stays an opt-in experiment (`TRY_GL=1`, `xmake vm --bridge --gl`); the default `dbus,gl=off` path is untouched by any of it, and the two fixes above are what make the guest's own GPU rendering possible at all.

Our own names, not the reference project's: the element, the import and the D-Bus side all get try-side names, and the code is written against the interfaces above rather than copied.

Do this only if measured: the guest is on llvmpipe either way, and the D-Bus path already sends damage rectangles rather than whole frames, so the copy cost tracks what changed.

## Order

0. dotfiles portability fixes (envs auto, monitors auto, foot `/usr/bin/zsh`, wallpaper path)
1. repo skeleton + submodule
2. firstBoot Go program + packages.txt
3. prebuilt QEMU boot
4. GPUI launcher
5. xmake.lua fetch + build
6. our own QEMU build, pinned by sha
7. GPU display: gpui half built (`vendor/gpui_apple`, `ui/src/guestSurface.rs`), QEMU half written as a readback and blocked on the libepoxy/ANGLE GL dispatch on macOS; measure before finishing
8. x86_64 + Linux later
