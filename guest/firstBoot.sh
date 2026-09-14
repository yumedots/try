#!/usr/bin/env bash
set -euo pipefail

uid_desktop=1000
share_name=${TRY_SHARE_NAME:-try}
state=/var/lib/try
dotfiles_share=/mnt/dotfiles
started=$(date +%s)

user=$(getent passwd "$uid_desktop" | cut -d: -f1)
if [ -z "$user" ]; then
    echo "no user with uid $uid_desktop, create one first" >&2
    exit 1
fi
home=$(getent passwd "$user" | cut -d: -f6)
share="$home/$share_name"
tarball="$share/.cache/archlinuxarm.tar.gz"
reset_flag="$share/build/resetConfig"

install -d -m 755 "$state" "$dotfiles_share"
install -d -o "$user" -g "$user" "$share"
mountpoint -q "$share" || mount -t 9p -o trans=virtio,version=9p2000.L share "$share"
mountpoint -q "$dotfiles_share" || mount -t 9p -o trans=virtio,version=9p2000.L dotfiles "$dotfiles_share" 2>/dev/null || true

here=$(cd "$(dirname "$0")" && pwd)
if [ "$here" != "$share/guest" ] && [ -f "$share/guest/firstBoot.sh" ]; then
    exec bash "$share/guest/firstBoot.sh"
fi

setup_shell () {
    printf 'export ZDOTDIR="$HOME/.config/zsh"\n' > "$home/.zshenv"
    if [ ! -d "$home/.oh-my-zsh" ] && command -v git >/dev/null; then
        git clone --depth 1 https://github.com/ohmyzsh/ohmyzsh "$home/.oh-my-zsh" || true
    fi
    chown "$user" "$home/.zshenv" 2>/dev/null || true
    if [ -d "$home/.oh-my-zsh" ]; then
        chown -R "$user" "$home/.oh-my-zsh"
    fi
    if [ -x /usr/bin/zsh ] && [ "$(getent passwd "$user" | cut -d: -f7)" != /usr/bin/zsh ]; then
        chsh -s /usr/bin/zsh "$user"
    fi
}

relink_config () {
    rm -rf "$home/.config"
    install -d -m 755 "$home/.config"
    if mountpoint -q "$dotfiles_share"; then
        for src in "$dotfiles_share"/*; do
            [ -e "$src" ] || continue
            ln -sfnT "$src" "$home/.config/$(basename "$src")"
        done
    else
        echo "dotfiles share not mounted, nothing to link (boot with: xmake run)" >&2
    fi
    chown -R "$user" "$home/.config"
    echo "linked $(find "$home/.config" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l) configs from $dotfiles_share"
    echo "mounts: $(mountpoint -q "$dotfiles_share" && echo dotfiles || echo dotfiles-missing), $(mountpoint -q "$share" && echo share || echo share-missing), fstab $(grep -c 9p /etc/fstab) 9p entries, zshrc $( [ -f "$home/.config/zsh/.zshrc" ] && echo ok || echo missing)"
}

wipe_home () {
    find "$home" -mindepth 1 -maxdepth 1 ! -name "$share_name" -exec rm -rf {} +
}

mode=""
if [ -e "$reset_flag" ]; then
    mode=$(cat "$reset_flag")
    rm -f "$reset_flag"
fi

provisioned=$state/provisioned
stamp=$(md5sum "$here/packages.txt" | cut -d' ' -f1)
if [ -s "$provisioned" ] && [ "$(cat "$provisioned")" = "$stamp" ]; then
    if [ "$mode" = "user" ]; then
        wipe_home
    fi
    relink_config
    setup_shell
    if [ -n "$mode" ]; then
        echo "$mode reset done in $(( $(date +%s) - started ))s"
    fi
    exit 0
fi

rm -f /var/lib/pacman/db.lck

if [ "$(stat -c %u /etc/passwd)" != "0" ]; then
    tar -xpf "$tarball" -C /
fi

if ! pacman -Qq >/dev/null 2>&1; then
    pacman-db-upgrade
fi

sed -i "s/^#\?[[:space:]]*ParallelDownloads.*/ParallelDownloads = 10/" /etc/pacman.conf

mirror_line='Server = http://ca.us.mirror.archlinuxarm.org/$arch/$repo'
if ! grep -qxF "$mirror_line" /etc/pacman.d/mirrorlist; then
    printf '%s\n' "$mirror_line" | cat - /etc/pacman.d/mirrorlist > /etc/pacman.d/mirrorlist.new
    mv /etc/pacman.d/mirrorlist.new /etc/pacman.d/mirrorlist
fi

for fstab_line in "share $share 9p trans=virtio,version=9p2000.L,nofail 0 0" \
                  "dotfiles $dotfiles_share 9p trans=virtio,version=9p2000.L,nofail 0 0"; do
    grep -qsxF "$fstab_line" /etc/fstab || printf '%s\n' "$fstab_line" >> /etc/fstab
done

waited=0
while [ "$waited" -lt 30 ] && ! ip route show default | grep -q .; do
    sleep 1
    waited=$((waited + 1))
done

if [ ! -s /etc/pacman.d/gnupg/pubring.gpg ]; then
    pacman-key --init
    pacman-key --populate archlinuxarm
fi

packages_started=$(date +%s)
listed=$(grep -vE '^[[:space:]]*(#|$)' "$here/packages.txt")
pkgs=$(printf '%s\n' "$listed" | grep -vx hyprland)
failed=""

if ! pacman -Sy --needed --noconfirm --noprogressbar $pkgs; then
    for pkg in $pkgs; do
        pacman -S --needed --noconfirm --noprogressbar "$pkg" || failed="$failed $pkg"
    done
fi

for pass in 1 2; do
    [ -n "$failed" ] || break
    upgrading=""
    for pkg in $failed; do
        pacman -S --needed --noconfirm --noprogressbar "$pkg" || upgrading="$upgrading $pkg"
    done
    if [ -n "$upgrading" ]; then
        echo "upgrading the system to satisfy:$upgrading" >&2
        pacman -Syu --needed --noconfirm --noprogressbar || true
    fi
    failed="$upgrading"
done

if printf '%s\n' "$listed" | grep -qx hyprland; then
    if ! pacman -S --needed --noconfirm --noprogressbar hyprland; then
        if ! pacman -S --needed --noconfirm --noprogressbar --assume-installed libaquamarine.so=13-64 hyprland; then
            failed="$failed hyprland"
        fi
    fi
fi
echo "packages in $(( $(date +%s) - packages_started ))s"

if [ -x /usr/bin/Hyprland ]; then
    for lib in $(ldd /usr/bin/Hyprland 2>/dev/null | awk '/not found/{print $1}'); do
        case "$lib" in
        libaquamarine.so.*)
            aqua=$(ls -1 /usr/lib/libaquamarine.so.* 2>/dev/null | grep -v "/$lib$" | tail -1 || true)
            if [ -n "$aqua" ]; then
                ln -sfn "$aqua" "/usr/lib/$lib"
            fi
            ;;
        esac
    done
    echo "Hyprland missing libs: $(ldd /usr/bin/Hyprland | grep -c 'not found')"
fi

systemctl enable --now systemd-networkd sshd

if command -v ly-dm >/dev/null; then
    printf 'LANG=C.UTF-8\n' > /etc/locale.conf
    systemctl disable getty@tty1.service 2>/dev/null || true
    systemctl enable ly@tty1.service
    echo "ly $(systemctl is-enabled ly@tty1.service), sessions: $(ls /usr/share/wayland-sessions 2>/dev/null | tr '\n' ' ')"
fi

if [ "$mode" = "user" ]; then
    wipe_home
fi
relink_config
setup_shell

if [ -n "$failed" ]; then
    echo "not installed:$failed after $(( $(date +%s) - started ))s" >&2
    echo "retry with: systemctl restart firstBoot.service" >&2
    exit 1
fi

printf '%s\n' "$stamp" > "$provisioned"
systemctl disable firstBoot.service 2>/dev/null || true
echo "provisioned in $(( $(date +%s) - started ))s"
