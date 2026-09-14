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

1. `xmake fetch`: QEMU + ISO -> `build/`, cached.
2. First boot: systemd once-unit runs the Go program from `guest/firstBoot/main.go`: install `guest/packages.txt`, link baked `dotfiles/*` -> `~/.config/*`, then disables itself.
3. Persistent disk: pay once, reuse every boot.
4. Dotfiles: copied into the guest image at build time. The guest home stays on the persistent disk.
5. QEMU: HVF accel, virtio-gpu virgl. Homebrew QEMU now, prebuilt per platform later.

## Resizable display architecture

The GPUI launcher will own the VM window and act as the display frontend instead of relying on QEMU Cocoa scaling.

1. Keep QEMU, ISO, and guest provisioning unchanged while the guest boots reliably.
2. Start QEMU with virtio-GPU and the D-Bus display backend, without a fixed `xres`/`yres`.
3. Add a GPUI display bridge that receives the guest framebuffer and forwards host window resize events to QEMU.
4. Let virtio-GPU notify the Linux guest, then let Wayland/Hyprland relayout at the new output size.
5. Keep a fixed-size fallback for hosts or QEMU builds without the display bridge.

The work order is: guest provisioning, QEMU D-Bus display export, GPUI framebuffer window, resize events, guest output verification, then fullscreen and fallback testing.

## xmake tasks

`doctor fetch guest run ui build package clean`. `is_os/is_arch` picks URLs. `depend.on_changed` skips re-fetch.

## Order

0. dotfiles portability fixes (envs auto, monitors auto, foot `/usr/bin/zsh`, wallpaper path)
1. repo skeleton + submodule
2. firstBoot Go program + packages.txt
3. prebuilt QEMU boot
4. GPUI launcher
5. xmake.lua fetch + build
6. x86_64 + Linux later
