#!/usr/bin/env python3
"""The Disks app's drives for a test (the design's four), as disks_data::Drive JSON: no real disk is ever read or
touched by a test. Its mounts are folders of the test area (the backups' one with files in it).
Usage: discos-teste.py OUT.json AREA_DIR"""
import json
import os
import sys

out, area = sys.argv[1], sys.argv[2]
backup = os.path.join(area, "Backup")
dados = os.path.join(area, "Dados")
for d in (backup, dados):
    os.makedirs(d, exist_ok=True)
for name in ("fotos-2025.tar", "documentos-2026.tar"):
    with open(os.path.join(backup, name), "wb") as f:
        f.write(os.urandom(64 * 1024))

GB = 1_000_000_000


def part(obj, dev, n, off, size, fs="", label="", name="", enc="", efi=False, mount=None, used=None):
    return {"object": obj, "device": dev, "number": n, "offset": off, "size": size, "fs": fs, "label": label, "name": name,
            "encrypted": enc, "efi": efi, "mount": mount, "used": used}


def drive(obj, dev, vendor, model, size, bus, rot, rem, health, parts):
    return {"object": obj, "device": dev, "vendor": vendor, "model": model, "size": size, "serial": "S6XNNS0T123456", "bus": bus,
            "rotational": rot, "removable": rem, "health": health, "parts": parts}


def health(wear=None, temp=None, hours=None):
    return {"smart": True, "failing": False, "warnings": [], "wear": wear, "temp_c": temp, "hours": hours, "bad_sectors": None}


U = "/org/freedesktop/UDisks2"
drives = [
    drive(f"{U}/drives/Samsung_990", "nvme0n1", "Samsung", "990 PRO", 1000 * GB, "nvme", False, False, health(4, 41.0, 1920), [
        part(f"{U}/block_devices/nvme0n1p1", "/dev/nvme0n1p1", 1, 1 << 20, 536_870_912, "vfat", "", "EFI System Partition", efi=True, mount="/boot/efi", used=40_000_000),
        part(f"{U}/block_devices/nvme0n1p2", "/dev/nvme0n1p2", 2, 600 << 20, 400 * GB, "ext4", "Ubuntu"),
        part(f"{U}/block_devices/nvme0n1p3", "/dev/nvme0n1p3", 3, 401 * GB, 599 * GB, "ext4", "KalangoOS", enc="LUKS2", mount="/", used=187 * GB),
    ]),
    drive(f"{U}/drives/Crucial_MX500", "sda", "", "CT2000MX500SSD1", 2000 * GB, "sata", False, False, health(None, 35.0, 3100), [
        part(f"{U}/block_devices/sda1", "/dev/sda1", 1, 1 << 20, 2000 * GB, "ext4", "Dados", mount=dados, used=1600 * GB),
    ]),
    drive(f"{U}/drives/WD_Blue", "sdb", "", "WDC WD5000LPCX", 500 * GB, "sata", True, False, health(None, 38.0, 14200), [
        part(f"{U}/block_devices/sdb1", "/dev/sdb1", 1, 1 << 20, 500 * GB, "ext4", "Backup", mount=backup, used=120 * GB),
    ]),
    drive(f"{U}/drives/Kingston_DataTraveler", "sdc", "Kingston", "DataTraveler 3.0", 64 * GB, "usb", False, True, health(), [
        part(f"{U}/block_devices/sdc1", "/dev/sdc1", 1, 1 << 20, 64 * GB, "vfat", "KINGSTON"),
    ]),
]
with open(out, "w") as f:
    json.dump(drives, f)
