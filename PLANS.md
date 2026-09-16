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

1. `xmake fetch`: the mirror is tested before it is used. `mirror.archlinuxarm.org` is only a redirector and is sticky per burst (12 parallel requests land on one backend, 12 sequential land on a mix), so it is asked 8 times in a row, each backend that answers is speed tested on a 2 MB range, and the fastest one that returns a real range is cached in `.cache/mirror.url` and used for the array download and its md5. A backend that serves the tarball's `.md5` as a 404 is what made the old path fall back to one slow stream (113 s); picking one measured mirror takes the same download to 84 s. QEMU bottles + Arch tarball -> `.cache/`, hash-pinned. `xmake clean --cache` chmods first, because Go's module cache is read-only and `rmdir` aborts on it.
2. First boot: systemd once-unit runs the Go program from `guest/firstBoot/main.go`: install `guest/packages.txt`, link baked `dotfiles/*` -> `~/.config/*`, then disables itself.
3. Persistent disk: pay once, reuse every boot.
4. Dotfiles: copied into the guest image at build time. The guest home stays on the persistent disk.
5. QEMU: HVF accel, virtio-gpu virgl. Homebrew QEMU now, prebuilt per platform later.

## Resizable display architecture

The window is QEMU's own Cocoa window (`-display cocoa,gl=es,zoom-to-fit=on`) and it is the master of the size; the guest follows it.

1. QEMU publishes the window's size in device pixels, the window in points described at 110 dpi, and the rate the display is refreshing at right now (measured here: 120 Hz ProMotion) into the guest's EDID. `xres`/`yres` are the boot mode and the ceiling.
2. The guest's display service (`guest/tryDisplay`) reads that EDID, builds a modeline plus a scale from it and applies it to Hyprland by name (`hyprctl eval`), so the desktop re-lays out at the window's size and one logical pixel is one host point.
3. Nothing resizes the window back. A guest that adopts the window's size is the normal case, so the window keeps the size the user made it; only a non-resizable window or a fullscreen window is fitted to the guest's mode.

A drag asks for one resize, not one per frame: `windowDidResize` sends UI info only when the drag has ended, and identical info is never sent twice.

What made the difference, measured: the rate the guest is told used to come from `CVDisplayLinkGetNominalOutputVideoRefreshPeriod`, which reports 12.6 Hz here, so Hyprland paced its whole desktop off a 12.65 Hz mode - the low frame rate and the lag that came with it. It now comes from `CGDisplayModeGetRefreshRate` on the display's current mode (`ui/cocoa.m`), and the guest's mode reads 2000x1336@119.998 Hz.

QEMU's own refresh cadence is the listener interval `1000000 / refresh_rate` milliseconds - 8 ms at 120 Hz - so the window is redrawn at ~115 Hz and QEMU is not the ceiling; the guest's renderer is (measured ~70 frames/s of guest damage while Hyprland was up).

The greeter is SDDM (`sddm` + `qt6-wayland`), not `ly`: a TUI greeter draws on the framebuffer console, whose framebuffer is allocated at the boot mode while the mode is the window's, so the console pans and the picture lands off-centre - everything `tryDisplay` used to do to `/dev/fb*` and to restart `ly` existed only to fight that. With SDDM the guest service only follows the EDID, and the console half of it is gone. The greeter is a Qt app, so it also gets the scale the desktop gets: Qt 6 on X11 ignores RandR physical DPI and takes a 96 dpi screen as 1x, so the whole device pixel ratio is the DPI the display is told. The service writes `/run/try/xft.dpi` and `/run/try/xsettingsd.conf` (`Xft/DPI` in 1024ths of it) for the scale the window rule landed on, and `guest/Xsetup` - which `tryDisplay` installs to `/usr/share/sddm/scripts/Xsetup`, where SDDM runs it as root just before the login dialog - starts `xsettingsd` on that display and merges the resource. That hook is the only moment that works, because the greeter's X server is started by SDDM itself, after `display-manager.service`; that is also why the unit is ordered `After=firstBoot.service` + `Before=display-manager.service`, and why `firstBoot` restarts SDDM with `--no-block` (waiting on that job would deadlock against `tryDisplay` starting). A scale change while the greeter is up is a `SIGHUP` to the daemon: `Xft/DPI` is the live channel - Qt's X11 plugin pushes it to every screen as a logical-DPI change - while the `Xft.dpi` resource is read once, when the plugin starts, so it only covers the first greeter and clients that never see the daemon.

A resize is applied to the compositor as a complete modeline taken from the EDID, not as "preferred": Aquamarine does not refresh its mode cache when an existing connector's EDID changes, so "preferred" resolves to the size the screen used to have. The modeline is built from the timing QEMU publishes - DisplayID first, because that is where QEMU puts anything past the base block's 12-bit fields - and the rule replaces the entry the guest's own monitor configuration made (`hl.monitor` inherits and overrides, and Hyprland matches rules last-first).

The physical-size fields carry the window in points at 110 dpi: 1x, 1.5x and 2x hosts land on Hyprland scales 1, 1.5 and 2. A size QEMU derived at its own 100 dpi fallback means no host described a window, and then the configured scale is left alone.

The request has to stay inside `xres`/`yres`. Past it the guest drops the new preferred mode (`virtio_gpu_conn_mode_valid` wants it within 16px of the last display info) and settles on the largest mode left: the window then shows a guest screen of another shape. `xmake vm` passes the host screen in device pixels, so that is both the boot mode and the ceiling.

The guest agent only nudges and reports: it writes `detect` to the connector so the kernel re-reads the EDID, logs the mode it applied (`window 2000x1336@120`, the rate read back out of the timing) and what the compositor ended up with (`compositor 2000x1336 scale 2`, or `does not fill`). It must never exit, because the connector may not exist yet when it starts. Resizing itself is QEMU's EDID plus the guest kernel, so it keeps working when the agent is not running.

`xmake vm --bridge` is the other route - QEMU's D-Bus display backend with the GPUI window as the frontend - and the GL readback below is what that route needed.

## GPU by host

`virtio-gpu-gl-pci` on both hosts, only accel and display backend differ.

- macOS arm64 Tahoe 26+: our fork's bottle (VirGL/ANGLE, `hvf`, `cocoa,gl=es`), pin + sha256 in `xmake.lua`, unpacked to `.cache/qemu`, missing Homebrew deps pulled from their own bottles, no brew install and no build. The guest's Mesa renders through virgl on the host GPU (`Renderer: virgl (ANGLE (Apple, Apple M3 Pro, ...))` in Hyprland's own log), and Cocoa blits the borrowed scanout texture.
- Linux: stock `qemu-system-*` from PATH, `kvm`, `gtk,gl=on`. Resize follows the window natively (upstream virtio-gpu `SetUIInfo`/EDID).
- `xmake vm --no-gl` falls back to a PATH QEMU and the software `virtio-gpu-pci` with `cocoa` (no `gl=es`).

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
- Our commits on it: the build workflow, then `53bbec375e` — the D-Bus listener reads a GL scanout back into a software surface on macOS and sends it as the same `Scanout`/`Update` pixels a 2D one is sent as, and `egl_init` asks ANGLE for its OpenGL backend when the display has no window (the cocoa frontend calls `qemu_egl_init_dpy_cocoa` itself, so its path is untouched) — then the display fixes below: `dbc50b19e8`, `dedd308a2d`, `3114cb59f9`, `e011829a3f`.
- Verified: fetch unpacks and patches the keg, `hvf` entitlement survives the patch step, deps resolve as before, and the guest boots on it (`tryDisplay: mode 3024x1964`). `-display cocoa,gl=es` cannot be checked on a headless runner, so the build proves GL by linkage instead.
- The cocoa window is the front end now: the mode it asks for is the window's pixels rounded to a multiple of 48, and the density it sends is picked so the guest's scale is the screen's own and never below it, growing only for windows larger than the one the app opens at — the desktop is retina sized in every window and zooms (same desktop, bigger pixels) as the window grows. Measured: 640x440pt → 1296x816 at scale 2, 1450x758pt → 2880x1440 at scale 2.67. The window is also given the size it was meant to open at, and the ui info is sent, once `applicationDidFinishLaunching` lets `updateUIInfo` through — before that it returns early, so the guest used to boot at the command-line resolution with the fallback density and render its login screen tiny until the first resize.

## GPU display (scoped, not built)

Today the default is `-display dbus,gl=on`: QEMU reads the guest's GL scanout back itself and sends it as the same `Scanout`/`Update` pixels a 2D one is sent as, so the guest renders through virgl on the host GPU while GPUI still draws ordinary frames (`TRY_NO_GL=1` selects `gl=off`). One half is still missing for a copy-free delivery:

- **QEMU:** its D-Bus listener delivers a GL scanout only under `CONFIG_GBM` (Linux dmabuf) or `WIN32` (D3D11 shared texture). On macOS that branch is empty. The texture borrowing in the patched tree is in-process only — it feeds the cocoa frontend, not a client across a socket. A macOS delivery needs a handle that survives a process boundary: an `IOSurface` id (`IOSurfaceLookup` is cross-process), published next to the scanout. Unknown until tried: whether the GL scanout is IOSurface-backed (ANGLE/Metal) at all.
- **try (done):** GPUI has no seam for a texture it does not own — `RenderImage` is `id` + `scale_factor` + CPU `Frame`s, `ImageSource` is `Resource`/`Render`/`Image`/`Custom`, and the macOS renderer uploads those frames into its own textures. What it does have is a macOS-only `surface()` element (`elements/surface.rs` + `Window::paint_surface`) that draws a caller-owned `CVPixelBuffer` with `CVMetalTextureCache` — the texture is made from the buffer's `IOSurface`, no copy — but the renderer asserted `kCVPixelFormatType_420YpCbCr8BiPlanarFullRange` (biplanar video), so a BGRA screen buffer could not use it.

  So: `vendor/gpui_apple` is the Apple renderer crate patched to take that second format. It is the *only* vendored crate; `gpui`, `gpui_macos`, `gpui_platform` stay the pinned zed rev, and `ui/Cargo.toml` redirects just it with `[patch."https://github.com/zed-industries/zed.git"]`. Layout: the four upstream sources verbatim, plus a checked-in `src/scene.h` (the cbindgen header for the pinned gpui, so the vendored build script needs no gpui checkout and no cbindgen) and our diff, which is the `surfaces_bgra_pipeline_state` pipeline plus one branch in `draw_surfaces` and a `surface_bgra_fragment` in `shaders.metal`. Regenerate `scene.h` from the pinned rev if the zed rev moves.

  try side: `ui/src/guestSurface.rs` — `GuestSurface` owns an IOSurface-backed BGRA `CVPixelBuffer` (`io_surface_id()` is the handle a QEMU delivery would hand back) and `guestSurface()` is the element. The frame arrives through `TRY_SURFACE=1` as a host-drawn pattern until the QEMU half exists; the check is `the_window_draws_the_callers_buffer`, which renders a scene through gpui's Metal headless renderer and asserts the read-back pixels come from the caller's buffer, then writes one pixel into the same buffer and asserts the next frame shows it — the second half is what a copy could not pass, and the whole test fails outright with the BGRA branch disabled.

  The other route, tried in the fork first because it needs no IOSurface and no client change: read the borrowed texture back with `glReadPixels` and send it through the ordinary `Scanout`/`Update` calls. It works as far as the D-Bus protocol goes — frames arrive — but the platform dies underneath it. On macOS libepoxy hands out the **system** OpenGL library's entry points, and those functions have no context of their own while ANGLE owns the rendering, so the first call segfaults. Measured in both places: our readback (`egl_fb_setup_for_tex` → `glGenFramebuffers`) and, before it, upstream's own `surface_gl_destroy_texture` → `glDeleteTextures`, reached when the guest resets the GPU during boot. Taking the entry points from the EGL display instead of libepoxy got past the first crash, and asking ANGLE for its OpenGL backend instead of Metal did not change which library the calls land in.

  Two things were fixed in the fork to get past both:

  1. **A virgl scanout of a 2D resource was rejected outright.** `virgl_cmd_set_scanout` called `virtio_gpu_check_scanout_bounds(..., 0, 0, ...)` — a zero-sized box, so *every* rect failed it. Upstream passes the resource's own width and height there. Measured on the guest: `[drm:virtio_gpu] *ERROR* response 0x1205 (command 0x103)` twice at every boot, `fb0` registered but no scanout set up, and QEMU's window saying `Display output not active` with the host console showing instead; after the fix the guest's mode is set (2560x1440) with no error, and the same boot on `-display cocoa,gl=es` has an output. That failure is also what reset the GPU (the reset path is where the GL crash was reached from), so it is what made the GL display die at every boot.

  2. **QEMU's GL calls went to the wrong library.** libepoxy resolves GL through the platform's GL — Apple's, which owns no context in our process — while the context belongs to ANGLE. Measured with `dladdr` on each pointer: `glDeleteTextures` and friends resolved to libepoxy (which then dlopened `libGL.dylib`), and the crash moved from `glDeleteTextures` to `glGetString` as each was fixed. `egl_init` now takes the entry points the display path uses — 55 of them, the display's calls and epoxy's own internal ones — from the EGL display, so they and the context are the same implementation. Crashes are gone.

  3. **The read back was black.** `glReadPixels` was asked for `GL_BGRA`, which GL ES has no format for, so the call failed with `GL_INVALID_ENUM` and left the surface as it was (`dbus: frame buffer read back failed`). It reads `GL_RGBA` now and swaps red and blue back into the software surface, which is BGRA. Measured on a booted guest over `dbus,gl=on`: the frames carry the guest's own picture (the greeter's header line reads back character for character), the FBO attaching the borrowed texture is complete (so virglrenderer's texture is visible in QEMU's context), and QEMU survives the whole boot. `xmake run` asks for the GL display now; `TRY_NO_GL=1` is the software path.  4. **The frame was upside down, and a frame that could not be read was sent anyway.** `glReadPixels` hands rows back bottom up, so a texture that says its first row is the top one — `VIRTIO_GPU_RESOURCE_FLAG_Y_0_TOP`, which is what the guest's virgl scanout carries (`y0top=1` on `tex=2`, against `y0top=0` on the 2D resources it uses in between, so the same screen can be upright at one moment and inverted after the next resize) — came out mirrored. Rows are turned over for that texture now. The read also used to make a context of its own current first, which cannot work here: `qemu_egl_display` is not initialized on this display (`eglMakeCurrent` → `EGL_NOT_INITIALIZED`) and a context of ours could not reach the borrowed texture anyway, while the renderer's own is current by the time it matters. A read that cannot happen — no context yet, or an incomplete frame buffer (`frame buffer is not complete (0x0)` twice at boot) — is reported once and the frame is left unsent, where before it was read into a surface nothing had written and that blank surface replaced the guest's picture: the screen went dark and stayed dark, because damage-driven content is never re-sent. Measured on the pinned CI build over `dbus,gl=on`, no window: frames at 2160x1440, 2000x1400, 1800x1200, 2560x1280 and 1120x1440 across five resizes, each upright and carrying content, the 2160x1440 one identical row for row to the same frame taken on `TRY_NO_GL=1`.

Our own names, not the reference project's: the element, the import and the D-Bus side all get try-side names, and the code is written against the interfaces above rather than copied.

Do this only if measured: the read back is a `glReadPixels` per damage rectangle rather than a copy-free handover, and the D-Bus path already sends damage rectangles rather than whole frames, so the cost tracks what changed.

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
