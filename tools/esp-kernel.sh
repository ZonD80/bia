#!/bin/bash
# Copy vmlinuz + initramfs out of a BIAROOT ext4 image onto DEST.
# That kernel always matches the rootfs arch (source/flash/EFI holds kernels for both ISAs).
set -euo pipefail
ROOTIMG=${1:?root.img}
DEST=${2:?dest dir}
mkdir -p "$DEST"
dir=$(cd "$(dirname "$ROOTIMG")" && pwd)
base=$(basename "$ROOTIMG")
dest=$(cd "$DEST" && pwd)
img=bia-build:arm64
plat=linux/arm64
if ! docker image inspect "$img" >/dev/null 2>&1; then
  img=bia-build:x86_64
  plat=linux/amd64
fi
docker image inspect "$img" >/dev/null 2>&1 || { echo "no bia-build image — make builder"; exit 1; }
echo "== kernel from $ROOTIMG -> $dest (match this rootfs, not flash/EFI)"
docker run --rm --privileged --platform "$plat" -v "$dir:/img" -v "$dest:/out" "$img" \
  sh -c "set -e
    mkdir -p /mnt
    mount -o loop,ro /img/$base /mnt
    v=\$(ls /mnt/boot/vmlinuz-* 2>/dev/null | grep -v '\\.old$' | head -1)
    i=\$(ls /mnt/boot/initrd.img-* /mnt/boot/initramfs* 2>/dev/null | grep -v '\\.old$' | head -1)
    [ -n \"\$v\" ] && [ -n \"\$i\" ]
    cp \"\$v\" /out/vmlinuz-lts
    cp \"\$i\" /out/initramfs-lts
    if [ -f /mnt/usr/local/share/bia/rescue.cpio.gz ]; then cp /mnt/usr/local/share/bia/rescue.cpio.gz /out/rescue.cpio.gz; fi
    umount /mnt"
# Keep the Debian kernel bytes intact (Secure Boot signatures). Do not unwrap zboot.
ls -lh "$dest/vmlinuz-lts" "$dest/initramfs-lts"
