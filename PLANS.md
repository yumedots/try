# try — plan

Goal: run this dotfiles desktop in a VM on Mac, like try-omarchy. No Docker, no QEMU build, no sign.

## Layout

```
try/
  dotfiles/      submodule -> dotfiles repo, at root
  guest/firstBoot.sh guest/packages.txt
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
2. First boot: systemd once-unit runs `guest/firstBoot.sh`: install `guest/packages.txt`, link `dotfiles/*` -> `~/.config/*`, then disables itself.
3. Persistent disk: pay once, reuse every boot.
4. 9p share: 1 Mac folder -> `~/SameName` in guest. Not full home.
5. QEMU: HVF accel, virtio-gpu virgl. Homebrew QEMU now, prebuilt per platform later.

## xmake tasks

`doctor fetch guest run ui build package clean`. `is_os/is_arch` picks URLs. `depend.on_changed` skips re-fetch.

## Order

0. dotfiles portability fixes (envs auto, monitors auto, foot `/usr/bin/zsh`, wallpaper path)
1. repo skeleton + submodule
2. firstBoot.sh + packages.txt
3. prebuilt QEMU boot
4. GPUI launcher
5. xmake.lua fetch + build
6. x86_64 + Linux later
