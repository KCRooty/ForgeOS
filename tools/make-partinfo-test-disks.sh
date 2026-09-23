#!/usr/bin/env bash
# Forge OS — genera dos discos de prueba para partinfo.rs:
# partinfo-mbr.img (tabla MBR clásica, 2 particiones) y
# partinfo-gpt.img (tabla GPT, 6 particiones — una por cada filesystem
# que partinfo.rs sabe reconocer).
#
# Usa tablas de particiones y filesystems REALES hechos por
# herramientas estándar de Linux (parted, mkfs.*), no bytes fabricados
# a mano — así la verificación demuestra que leemos el formato de
# verdad, no que somos consistentes con nosotros mismos.
#
# Requiere: parted, gdisk (paquete gdisk, solo por si hace falta en
# algún sistema), dosfstools (mkfs.vfat), e2fsprogs (mkfs.ext2/3/4),
# ntfs-3g (mkfs.ntfs), btrfs-progs (mkfs.btrfs), y permiso para usar
# losetup (con offset+sizelimit, sin necesitar montar nada).
# En CachyOS/Arch: `yay -S parted gptfdisk dosfstools e2fsprogs
# ntfs-3g btrfs-progs`.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

fmt_partition() {
    local img="$1" off="$2" size="$3" fs="$4" label="$5"
    local loop
    loop="$(losetup -o "$off" --sizelimit "$size" -f --show "$img")"
    case "$fs" in
        ext2) mkfs.ext2 -q -F -L "$label" "$loop" ;;
        ext3) mkfs.ext3 -q -F -L "$label" "$loop" ;;
        ext4) mkfs.ext4 -q -F -L "$label" "$loop" ;;
        vfat) mkfs.vfat -F 32 -n "$label" "$loop" > /dev/null ;;
        ntfs) mkfs.ntfs -F -Q -L "$label" "$loop" > /dev/null ;;
        btrfs) mkfs.btrfs -q -f -L "$label" "$loop" > /dev/null ;;
        *) echo "fs desconocido: $fs" >&2; losetup -d "$loop"; exit 1 ;;
    esac
    losetup -d "$loop"
}

echo "[1/2] partinfo-mbr.img — MBR con 2 particiones (ext4 + fat32)..."
MBR="$ROOT/partinfo-mbr.img"
rm -f "$MBR"
truncate -s 48M "$MBR"
parted -s "$MBR" mklabel msdos 2>/dev/null
parted -s "$MBR" unit MiB mkpart primary ext4 1 17 2>/dev/null
parted -s "$MBR" unit MiB mkpart primary fat32 17 33 2>/dev/null
fmt_partition "$MBR" 1048576 16777216 ext4 MBRP1EXT4
fmt_partition "$MBR" 17825792 16777216 vfat MBRP2FAT

echo "[2/2] partinfo-gpt.img — GPT con 6 particiones (ext2/ext3/ext4/fat32/ntfs/btrfs)..."
GPT="$ROOT/partinfo-gpt.img"
rm -f "$GPT"
truncate -s 400M "$GPT"
parted -s "$GPT" mklabel gpt 2>/dev/null
parted -s "$GPT" unit MiB mkpart ext2part ext2 1 33 2>/dev/null
parted -s "$GPT" unit MiB mkpart ext3part ext3 33 65 2>/dev/null
parted -s "$GPT" unit MiB mkpart ext4part ext4 65 97 2>/dev/null
parted -s "$GPT" unit MiB mkpart fatpart fat32 97 129 2>/dev/null
parted -s "$GPT" unit MiB mkpart ntfspart ntfs 129 193 2>/dev/null
# btrfs exige un mínimo de ~109 MiB por dispositivo — 128 MiB de margen
parted -s "$GPT" unit MiB mkpart btrfspart btrfs 193 321 2>/dev/null
fmt_partition "$GPT" 1048576 33554432 ext2 GPTP1EXT2
fmt_partition "$GPT" 34603008 33554432 ext3 GPTP2EXT3
fmt_partition "$GPT" 68157440 33554432 ext4 GPTP3EXT4
fmt_partition "$GPT" 101711872 33554432 vfat GPTP4FAT
fmt_partition "$GPT" 135266304 67108864 ntfs GPTP5NTFS
fmt_partition "$GPT" 202375168 134217728 btrfs GPTP6BTRFS

echo ""
echo "Listo: $MBR y $GPT"
echo "Arranca cualquiera de los dos con: cp <imagen> \"$ROOT/disk.img\" && ./tools/run-qemu.sh"
echo "Desde la consola: partinfo"
