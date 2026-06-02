#!/usr/bin/env bash
# Build and install the nitro CLI + nitrod daemon, then enable the service.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> building release binaries..."
cargo build --release --bin nitro --bin nitrod --bin nitro-gui

echo "==> installing binaries to /usr/local/bin (requires sudo)..."
sudo install -Dm755 target/release/nitro     /usr/local/bin/nitro
sudo install -Dm755 target/release/nitrod    /usr/local/bin/nitrod
sudo install -Dm755 target/release/nitro-gui /usr/local/bin/nitro-gui

echo "==> installing desktop launcher + icon..."
sudo install -Dm644 data/desktop/nitro-gui.desktop /usr/share/applications/nitro-gui.desktop
sudo install -Dm644 data/desktop/nitro-gui.svg /usr/share/icons/hicolor/scalable/apps/nitro-gui.svg
sudo update-desktop-database /usr/share/applications 2>/dev/null || true
sudo gtk-update-icon-cache -f /usr/share/icons/hicolor 2>/dev/null || true

echo "==> installing polkit rule (no password prompts for the local desktop user)..."
sudo install -Dm644 data/polkit/49-nitro.rules /etc/polkit-1/rules.d/49-nitro.rules
# Optional group so non-local/SSH sessions can be authorised too; the local
# desktop session is already covered by the rule without any group membership.
sudo groupadd -f nitro
sudo usermod -aG nitro "${SUDO_USER:-$(id -un)}" || true

echo "==> installing systemd unit..."
sudo install -Dm644 data/systemd/nitrod.service /etc/systemd/system/nitrod.service

echo "==> creating default config (/etc/nitro/config.json) if absent..."
if [[ ! -f /etc/nitro/config.json ]]; then
  sudo install -d /etc/nitro
  sudo tee /etc/nitro/config.json >/dev/null <<'JSON'
{
  "profile": "balanced",
  "auto_switch": false,
  "charge_limit": null,
  "thermal_guard_c": 95.0,
  "hotkey_device": null,
  "hotkey_code": null,
  "poll_secs": 5
}
JSON
fi

echo "==> enabling and starting nitrod..."
sudo systemctl daemon-reload
sudo systemctl enable --now nitrod

echo
echo "installed. the desktop app no longer asks for a password on each change."
echo "try:"
echo "  nitro-gui                    # graphical control panel (or launch 'NitroSense' from your apps)"
echo "  nitro status                 # mode + daemon + live sensors"
echo "  nitro profile performance    # or: quiet | balanced | turbo"
echo "  nitro auto on                # auto-switch by AC/thermal"
echo "  nitro learn-key              # teach it the NitroSense key"
echo "  nitro monitor                # live stream"
