#!/usr/bin/env bash
# Build and install the linuwu-sense kernel module, which exposes the Acer
# gaming-WMI interface (fan speed, keyboard RGB, thermal profiles, battery
# limiter) as validated sysfs attributes. These are firmware-mediated WMI
# methods -- the same the official NitroSense uses -- not raw EC writes.
#
# The Acer Nitro AN515-56 (this project's target) is not in linuwu-sense's
# built-in DMI table, so we patch in a quirk that mirrors the whitelisted
# AN515-55 sibling (nitro_sense => fan_speed + battery_limiter) and adds the
# four-zone keyboard flag (four_zone_kb => 4-zone RGB via WMI method 6). RGB on
# this model is the firmware WMI backlight, reached through the same confirmed
# Acer gaming-WMI GUID; the USB HID keyboard only carries keystrokes. An
# unsupported WMI method returns an error rather than touching a raw register,
# so enabling the quirk is safe to try.
#
# This replaces the in-tree acer_wmi module with a patched build, packaged via
# DKMS so it is rebuilt automatically on every future kernel upgrade. It needs
# matching kernel headers, sudo and network access. Run it yourself:
#   ./scripts/install-kernel-module.sh
#
# Uninstall later with:
#   sudo dkms remove -m linuwu-sense -v 1.0 --all
#   sudo rm -f /etc/modules-load.d/linuwu_sense.conf /etc/modprobe.d/blacklist-acer_wmi.conf
set -euo pipefail

REPO="https://github.com/0x7375646F/Linuwu-Sense.git"
SRC="${1:-$HOME/.cache/linuwu-sense}"

echo ">> linuwu-sense installer"
echo "   source dir: $SRC"
echo "   kernel    : $(uname -r)"
echo

# 1. Toolchain + headers (Fedora / dnf). On other distros install the
#    equivalents manually: kernel headers, make, gcc, git, python3.
if command -v dnf >/dev/null 2>&1; then
    echo ">> Installing build prerequisites via dnf (sudo)..."
    sudo dnf install -y "kernel-devel-$(uname -r)" kernel-headers make gcc git python3 dkms \
        || echo "!! Could not install some packages; continuing if already present."
else
    echo "!! dnf not found. Ensure kernel headers, make, gcc, git, python3 and dkms are installed."
fi

# 2. Fetch or update the source. Reset any prior local patch first so the
#    fast-forward pull (and a fresh re-patch) always succeed.
if [ -d "$SRC/.git" ]; then
    echo ">> Updating existing checkout..."
    git -C "$SRC" checkout -- . 2>/dev/null || true
    git -C "$SRC" pull --ff-only || echo "!! git pull failed; using existing checkout."
else
    echo ">> Cloning $REPO ..."
    git clone --depth 1 "$REPO" "$SRC"
fi

# 3. Patch in the Acer Nitro AN515-56 DMI quirk (idempotent). This adds a quirk
#    struct { .nitro_sense = 2, .four_zone_kb = 1 } plus a DMI match, mirroring
#    the AN515-55 legacy Nitro and additionally enabling the four-zone keyboard
#    group. Firmware-mediated WMI: an unsupported method errors out instead of
#    writing an unknown register, so this is safe to try on the real hardware.
patch_source() {
    local f="$SRC/src/linuwu_sense.c"
    [ -f "$f" ] || { echo "!! source not found: $f"; return 1; }
    echo ">> Patching DMI table: add Acer Nitro AN515-56 quirk (fan + battery + RGB)."
    python3 - "$f" <<'PYEOF'
import re, sys
path = sys.argv[1]
data = open(path, encoding="utf-8", errors="surrogateescape").read()

if "AN515-56" in data:
    print("   already patched; skipping.")
    sys.exit(0)

# 1) quirk struct, inserted right after quirk_acer_nitro_legacy.
struct_re = re.compile(
    r"(static\s+struct\s+quirk_entry\s+quirk_acer_nitro_legacy\s*=\s*\{[^}]*\}\s*;)",
    re.DOTALL)
new_struct = (
    "\n\nstatic struct quirk_entry quirk_acer_nitro_an515_56 = {\n"
    "    .nitro_sense = 2,\n"
    "    .four_zone_kb = 1,\n"
    "};")
data, n1 = struct_re.subn(lambda m: m.group(1) + new_struct, data, count=1)
if n1 != 1:
    sys.exit("   ERROR: quirk_acer_nitro_legacy anchor not found")

# 2) DMI table entry, inserted right before the AN515-55 entry.
dmi_re = re.compile(
    r"(\{\s*\.callback\s*=\s*dmi_matched\s*,\s*"
    r"\.ident\s*=\s*\"Acer Nitro AN515-55\"\s*,.*?"
    r"\.driver_data\s*=\s*&quirk_acer_nitro_legacy\s*,\s*\}\s*,)",
    re.DOTALL)
new_dmi = (
    "    {\n"
    "        .callback = dmi_matched,\n"
    "        .ident = \"Acer Nitro AN515-56\",\n"
    "        .matches = {\n"
    "            DMI_MATCH(DMI_SYS_VENDOR, \"Acer\"),\n"
    "            DMI_MATCH(DMI_PRODUCT_NAME, \"Nitro AN515-56\"),\n"
    "        },\n"
    "        .driver_data = &quirk_acer_nitro_an515_56,\n"
    "    },\n")
data, n2 = dmi_re.subn(lambda m: new_dmi + m.group(1), data, count=1)
if n2 != 1:
    sys.exit("   ERROR: AN515-55 DMI entry anchor not found")

open(path, "w", encoding="utf-8", errors="surrogateescape").write(data)
print("   patched: quirk_acer_nitro_an515_56 + DMI entry added.")
PYEOF
}
patch_source

# A previous version of this script forced capabilities with a modprobe option.
# The DMI quirk supersedes it (and additionally enables RGB), so drop any stale
# force_caps drop-in to avoid a confusing capability override.
if [ -f /etc/modprobe.d/linuwu_sense.conf ] \
   && grep -q force_caps /etc/modprobe.d/linuwu_sense.conf 2>/dev/null; then
    echo ">> Removing stale force_caps drop-in (superseded by the DMI quirk)."
    sudo rm -f /etc/modprobe.d/linuwu_sense.conf
fi

# 4. Package the patched source with DKMS so the module is rebuilt automatically
#    on every future kernel upgrade (AUTOINSTALL=yes) -- no need to re-run this.
DKMS_NAME="linuwu-sense"
DKMS_VER="1.0"
DKMS_SRC="/usr/src/${DKMS_NAME}-${DKMS_VER}"

echo ">> Installing patched source into $DKMS_SRC ..."
( cd "$SRC" && sudo make clean >/dev/null 2>&1 || true )
sudo rm -rf "$DKMS_SRC"
sudo mkdir -p "$DKMS_SRC"
sudo cp -a "$SRC/." "$DKMS_SRC/"
sudo rm -rf "$DKMS_SRC/.git" "$DKMS_SRC/.github"

echo ">> Writing dkms.conf ..."
sudo tee "$DKMS_SRC/dkms.conf" >/dev/null <<EOF
PACKAGE_NAME="$DKMS_NAME"
PACKAGE_VERSION="$DKMS_VER"
AUTOINSTALL="yes"
MAKE[0]="make KVER=\${kernelver}"
CLEAN="make KVER=\${kernelver} clean"
BUILT_MODULE_NAME[0]="linuwu_sense"
BUILT_MODULE_LOCATION[0]="src"
DEST_MODULE_LOCATION[0]="/updates/dkms"
EOF

echo ">> Registering + building with DKMS ..."
# Drop any previous registration so a re-run always tracks the latest source.
sudo dkms remove  -m "$DKMS_NAME" -v "$DKMS_VER" --all 2>/dev/null || true
sudo dkms add     -m "$DKMS_NAME" -v "$DKMS_VER"
sudo dkms build   -m "$DKMS_NAME" -v "$DKMS_VER"
sudo dkms install -m "$DKMS_NAME" -v "$DKMS_VER" --force

# 4a. Keep the in-tree acer_wmi out of the way and load linuwu_sense at boot.
#     (DKMS owns the .ko build/refresh; these two drop-ins handle binding + boot.)
echo ">> Ensuring acer_wmi blacklist + load-at-boot drop-ins ..."
echo "blacklist acer_wmi" | sudo tee /etc/modprobe.d/blacklist-acer_wmi.conf >/dev/null
echo "linuwu_sense"       | sudo tee /etc/modules-load.d/linuwu_sense.conf   >/dev/null

# 4b. Remove any older hand-built .ko for the running kernel so the DKMS copy is
#     authoritative, then reload so the freshly built quirk takes effect now.
echo ">> Removing stale hand-built module (if any) + reloading ..."
sudo rm -f "/lib/modules/$(uname -r)/kernel/drivers/platform/x86/linuwu_sense.ko"
sudo depmod -a
sudo modprobe -r linuwu_sense 2>/dev/null || true
sudo modprobe -r acer_wmi 2>/dev/null || true
sudo modprobe linuwu_sense

# 5. Verify the sysfs surface appeared.
BASE="/sys/module/linuwu_sense/drivers/platform:acer-wmi/acer-wmi"
ALT="/sys/devices/platform/acer-wmi"

nitro_iface_present() {
    [ -e "$BASE/nitro_sense/fan_speed" ] || [ -e "$ALT/nitro_sense/fan_speed" ] \
        || [ -e "$BASE/predator_sense/fan_speed" ] || [ -e "$ALT/predator_sense/fan_speed" ]
}
rgb_iface_present() {
    [ -e "$BASE/four_zoned_kb/per_zone_mode" ] || [ -e "$ALT/four_zoned_kb/per_zone_mode" ]
}

# Fallback: if the quirk somehow did not take (e.g. an unexpected product_name),
# enable at least fan + battery via force_caps for the verified AN515-56. RGB
# needs the DMI quirk compiled in, so it stays off on this fallback path.
enable_force_caps_fallback() {
    local product conf="/etc/modprobe.d/linuwu_sense.conf"
    product=$(cat /sys/class/dmi/id/product_name 2>/dev/null || echo unknown)
    if [ "$product" = "Nitro AN515-56" ]; then
        echo ">> Quirk did not expose the interface; trying force_caps=10240"
        echo "   (fan_speed + battery_limiter only; RGB requires the DMI quirk)."
        echo "options linuwu_sense force_caps=10240" | sudo tee "$conf" >/dev/null
        sudo modprobe -r linuwu_sense 2>/dev/null || true
        sudo modprobe linuwu_sense
    else
        echo "!! '$product' is not the verified AN515-56 target; not forcing caps."
        echo "   Map your model into the DMI table (see patch_source in this script)."
    fi
}

echo
if nitro_iface_present; then
    echo ">> Success: the Nitro WMI interface is live."
else
    echo ">> Interface absent after build; attempting fallback..."
    enable_force_caps_fallback
fi

echo
if nitro_iface_present; then
    echo ">> Available Acer WMI attributes:"
    for b in "$BASE" "$ALT"; do
        for d in nitro_sense predator_sense four_zoned_kb; do
            [ -d "$b/$d" ] && echo "   - $b/$d"
        done
    done
    echo
    echo "Fan + battery limit (firmware WMI):"
    echo "   nitro fan max        # then: nitro fan auto"
    echo "   nitro charge-limit 80"
    echo "   nitro status"
    if rgb_iface_present; then
        echo
        echo "Keyboard RGB (4-zone, firmware WMI method 6):"
        echo "   nitro rgb ff0000                              # all zones red"
        echo "   nitro rgb zones ff0000 00ff00 0000ff ffff00  # per-zone"
        echo "   nitro rgb off"
    else
        echo
        echo "Note: fan/battery are live but the four_zoned_kb group did not appear."
        echo "      Check 'sudo dmesg | grep -i acer' and re-run; RGB needs the quirk."
    fi
else
    echo "!! Nitro interface still absent. A reboot may be needed for the patched"
    echo "   acer_wmi to load cleanly; re-run this script or check 'sudo dmesg | grep -i acer'."
fi
