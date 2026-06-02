#!/usr/bin/env bash
# Stop/disable the daemon, restore safe CPU defaults, and remove installed files.
set -euo pipefail

echo "==> stopping and disabling nitrod..."
sudo systemctl disable --now nitrod 2>/dev/null || true
# ExecStopPost already restored balanced defaults; run it explicitly too in case
# the unit was already gone.
/usr/local/bin/nitrod --restore 2>/dev/null || sudo /usr/local/bin/nitrod --restore 2>/dev/null || true

echo "==> removing files..."
sudo rm -f /etc/systemd/system/nitrod.service
sudo rm -f /etc/polkit-1/rules.d/49-nitro.rules
sudo groupdel nitro 2>/dev/null || true
sudo rm -f /usr/local/bin/nitro /usr/local/bin/nitrod /usr/local/bin/nitro-gui
sudo rm -f /usr/share/applications/nitro-gui.desktop
sudo rm -f /usr/share/icons/hicolor/scalable/apps/nitro-gui.svg
sudo update-desktop-database /usr/share/applications 2>/dev/null || true
sudo rm -rf /run/nitro
sudo systemctl daemon-reload

echo "==> config left at /etc/nitro (remove manually if desired: sudo rm -rf /etc/nitro)"
echo "uninstalled. CPU returned to balanced defaults; fans were never modified."
