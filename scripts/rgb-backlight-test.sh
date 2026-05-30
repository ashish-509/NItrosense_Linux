#!/usr/bin/env bash
#
# rgb-backlight-test.sh — figure out whether the keyboard backlight physically
# lights, and via which WMI path.
#
# Background: the linuwu-sense per-zone write path first sets the *global*
# keyboard colour to static-black (WMI method 20) and only then writes per-zone
# colours (method 6). On some AN515-56 units the keyboard shows the global
# colour, so per-zone writes succeed in sysfs yet leave the keys dark. This
# script A/B-tests the four-zone STATIC path (global colour set directly) against
# the per-zone path, so we can tell which one — if either — actually produces
# light on THIS machine.
#
# It only ever writes the two documented, firmware-validated sysfs attributes
# (four_zone_mode / per_zone_mode). No raw EC, no guessed registers. It backs up
# the current state and restores it on exit.
#
# Usage:  sudo ./scripts/rgb-backlight-test.sh
#
set -euo pipefail

# --- locate the four_zoned_kb directory (same logic as nitro-hal::acer) --------
KBD_DIR=""
for base in \
  /sys/module/linuwu_sense/drivers/platform:acer-wmi/acer-wmi \
  /sys/devices/platform/acer-wmi; do
  if [[ -e "$base/four_zoned_kb/per_zone_mode" ]]; then
    KBD_DIR="$base/four_zoned_kb"
    break
  fi
done

if [[ -z "$KBD_DIR" ]]; then
  echo "ERROR: four_zoned_kb sysfs group not found."
  echo "  The linuwu-sense module with the AN515-56 quirk must be loaded."
  echo "  Run ./scripts/install-kernel-module.sh first."
  exit 1
fi

if [[ "$(id -u)" -ne 0 ]]; then
  echo "This test writes sysfs and reads dmesg; please run it with sudo:"
  echo "  sudo $0"
  exit 1
fi

FOUR="$KBD_DIR/four_zone_mode"
PER="$KBD_DIR/per_zone_mode"

echo "Using: $KBD_DIR"
echo

# --- back up current state and arrange to restore it ---------------------------
ORIG_FOUR="$(cat "$FOUR" 2>/dev/null || echo '0,0,100,0,0,0,0')"
ORIG_PER="$(cat "$PER"  2>/dev/null || echo '000000,000000,000000,000000,100')"
echo "Saved current state:"
echo "  four_zone_mode = $ORIG_FOUR"
echo "  per_zone_mode  = $ORIG_PER"
echo

restore() {
  echo
  echo ">> Restoring your previous RGB state..."
  echo "$ORIG_FOUR" > "$FOUR" 2>/dev/null || true
  # Only restore per_zone if the zones actually differed (else four_zone wins).
  echo "$ORIG_PER" > "$PER" 2>/dev/null || true
}
trap restore EXIT

pause() { read -rp "   $1 [press Enter] " _ || true; }

write_four() {  # mode,speed,brightness,dir,r,g,b
  echo "$1" > "$FOUR"
}

echo "=============================================================="
echo " TEST A — four_zone STATIC path (WMI method 20, the new fix)"
echo " This sets the GLOBAL backlight colour directly."
echo "=============================================================="
for entry in "RED:0,0,100,0,255,0,0" "GREEN:0,0,100,0,0,255,0" "BLUE:0,0,100,0,0,0,255" "WHITE:0,0,100,0,255,255,255"; do
  name="${entry%%:*}"; val="${entry#*:}"
  echo ">> Setting whole keyboard $name  ($val)"
  write_four "$val"
  pause "Look at the keyboard. Is it lit $name now?"
done

echo
echo "=============================================================="
echo " TEST B — brightness sweep on four_zone static (is it dimming?)"
echo "=============================================================="
for b in 100 50 10 0; do
  echo ">> White at brightness ${b}%"
  write_four "0,0,${b},0,255,255,255"
  pause "Did the brightness visibly change?"
done

echo
echo "=============================================================="
echo " TEST C — per_zone path (old path: method 20 black + method 6)"
echo "=============================================================="
echo ">> Writing per_zone all-RED at brightness 100"
echo "ff0000,ff0000,ff0000,ff0000,100" > "$PER"
pause "Is the keyboard lit RED now (per-zone path)?"

echo
echo "=============================================================="
echo " Kernel log from these writes (look for errors)"
echo "=============================================================="
dmesg 2>/dev/null | grep -iE "linuwu|acer|kb.*status|zone|backlight|rgb" | tail -25 \
  || echo "(no matching dmesg lines)"

echo
echo "=============================================================="
echo " RESULT — please note which test(s) produced VISIBLE light:"
echo "   * TEST A lit  -> the four_zone-static fix works; done."
echo "   * only TEST C -> keep per_zone; tell me and I'll revert the routing."
echo "   * NOTHING lit + BIOS/Windows also never lights the keys"
echo "                 -> this unit has no controllable backlight (hardware)."
echo "=============================================================="
# state is restored by the EXIT trap
