#!/usr/bin/env bash
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
uid_desktop=1000
share_name=${TRY_SHARE_NAME:-try}
state=/var/lib/try
provisioned=$state/provisioned

user=$(getent passwd "$uid_desktop" | cut -d: -f1)
if [ -z "$user" ]; then
    echo "no user with uid $uid_desktop, create one first" >&2
    exit 1
fi
home=$(getent passwd "$user" | cut -d: -f6)
share="$home/$share_name"
tarball="$share/build/archlinuxarm.tar.gz"

stamp=$(md5sum "$here/firstBoot.sh" | cut -d' ' -f1)
install -d -m 755 "$state"
if [ -s "$provisioned" ] && [ "$(cat "$provisioned")" = "$stamp" ]; then
    exit 0
fi

install -d -o "$user" -g "$user" "$share"
mountpoint -q "$share" || mount -t 9p -o trans=virtio,version=9p2000.L share "$share"

if [ "$(stat -c %u /etc/passwd)" != "0" ]; then
    tar -xpf "$tarball" -C /
fi

if ! pacman -Qq >/dev/null 2>&1; then
    pacman-db-upgrade
fi

ignore=$(pacman -Qq | grep -E '^linux-firmware|^linux-aarch64$' | tr '\n' ' ')
sed -i "s/^#\?[[:space:]]*ParallelDownloads.*/ParallelDownloads = 10/;s/^#\?[[:space:]]*IgnorePkg.*/IgnorePkg = $ignore/" /etc/pacman.conf

fstab_line="share $share 9p trans=virtio,version=9p2000.L,nofail 0 0"
grep -qsxF "$fstab_line" /etc/fstab || printf '%s\n' "$fstab_line" >> /etc/fstab

if [ ! -s /etc/pacman.d/gnupg/pubring.gpg ]; then
    pacman-key --init
    pacman-key --populate archlinuxarm
fi

listed=$(grep -vE '^[[:space:]]*(#|$)' "$here/packages.txt")
pkgs=$(printf '%s\n' "$listed" | grep -vx hyprland)
failed=""

if ! pacman -Syu --needed --noconfirm --noprogressbar $pkgs; then
    for pkg in $pkgs; do
        pacman -S --needed --noconfirm --noprogressbar "$pkg" || failed="$failed $pkg"
    done
fi

if printf '%s\n' "$listed" | grep -qx hyprland; then
    if ! pacman -S --needed --noconfirm --noprogressbar hyprland; then
        if ! pacman -S --needed --noconfirm --noprogressbar --assume-installed libaquamarine.so=13-64 hyprland; then
            failed="$failed hyprland"
        fi
    fi
fi

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

install -d -m 755 "$home/.config"
for src in "$share/dotfiles"/*; do
    ln -sfnT "$src" "$home/.config/$(basename "$src")"
done
chown -R "$user" "$home/.config"

if [ ! -d "$home/.oh-my-zsh" ]; then
    git clone --depth 1 https://github.com/ohmyzsh/ohmyzsh "$home/.oh-my-zsh" || true
fi
printf 'export ZDOTDIR="$HOME/.config/zsh"\n' > "$home/.zshenv"
chown -R "$user" "$home/.oh-my-zsh" "$home/.zshenv"
[ -x /usr/bin/zsh ] && chsh -s /usr/bin/zsh "$user"

if [ -n "$failed" ]; then
    echo "not installed:$failed" >&2
    echo "retry with: systemctl restart firstBoot.service" >&2
    exit 1
fi

printf '%s\n' "$stamp" > "$provisioned"
systemctl disable firstBoot.service 2>/dev/null || true
