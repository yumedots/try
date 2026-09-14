#!/usr/bin/env bash
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
uid_desktop=1000
share_name=${TRY_SHARE_NAME:-try}

user=$(getent passwd "$uid_desktop" | cut -d: -f1)
if [ -z "$user" ]; then
    echo "no user with uid $uid_desktop, create one first" >&2
    exit 1
fi
home=$(getent passwd "$user" | cut -d: -f6)
share="$home/$share_name"
tarball="$share/build/archlinuxarm.tar.gz"

install -d -o "$user" -g "$user" "$share"
mountpoint -q "$share" || mount -t 9p -o trans=virtio,version=9p2000.L share "$share"

if [ "$(stat -c %u /etc/passwd)" != "0" ]; then
    tar -xpf "$tarball" -C /
fi

fstab_line="share $share 9p trans=virtio,version=9p2000.L,nofail 0 0"
grep -qsxF "$fstab_line" /etc/fstab || printf '%s\n' "$fstab_line" >> /etc/fstab

pacman-key --init
pacman-key --populate archlinuxarm
pacman -Syu --needed --noconfirm $(grep -vE '^[[:space:]]*(#|$)' "$here/packages.txt")
systemctl enable --now systemd-networkd sshd

install -d -m 755 "$home/.config"
for src in "$share/dotfiles"/*; do
    ln -sfnT "$src" "$home/.config/$(basename "$src")"
done
chown -R "$user" "$home/.config"
chsh -s /usr/bin/zsh "$user"

systemctl disable firstBoot.service 2>/dev/null || true
