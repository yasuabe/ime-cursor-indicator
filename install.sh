#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUST_DIR="$SCRIPT_DIR/rust"
SOURCE_CONFIG="$SCRIPT_DIR/config.toml.example"
SOURCE_BINARY="$RUST_DIR/target/release/ime-cursor-indicator"

TARGET_BIN_DIR="$HOME/.local/bin"
TARGET_BINARY="$TARGET_BIN_DIR/ime-cursor-indicator"
TARGET_CONFIG_DIR="$HOME/.config/ime-cursor-indicator"
TARGET_CONFIG="$TARGET_CONFIG_DIR/config.toml"
AUTOSTART_DIR="$HOME/.config/autostart"
AUTOSTART_FILE="$AUTOSTART_DIR/ime-cursor-indicator.desktop"

log() {
  printf '[install] %s\n' "$1"
}

fail() {
  printf '[install] ERROR: %s\n' "$1" >&2
  exit 1
}

main() {
  log "Building release binary in $RUST_DIR"
  (
    cd "$RUST_DIR"
    cargo build --release
  )

  [[ -f "$SOURCE_BINARY" ]] || fail "Built binary not found at $SOURCE_BINARY"
  [[ -f "$SOURCE_CONFIG" ]] || fail "Example config not found at $SOURCE_CONFIG"

  log "Installing binary to $TARGET_BINARY"
  mkdir -p "$TARGET_BIN_DIR"
  install -m 755 "$SOURCE_BINARY" "$TARGET_BINARY"
  binary_status="installed"

  log "Ensuring config directory exists at $TARGET_CONFIG_DIR"
  mkdir -p "$TARGET_CONFIG_DIR"

  if [[ -f "$TARGET_CONFIG" ]]; then
    log "Keeping existing config at $TARGET_CONFIG"
    config_status="kept existing config"
  else
    log "Copying default config to $TARGET_CONFIG"
    install -m 644 "$SOURCE_CONFIG" "$TARGET_CONFIG"
    config_status="created default config"
  fi

  log "Writing autostart desktop entry to $AUTOSTART_FILE"
  mkdir -p "$AUTOSTART_DIR"
  cat >"$AUTOSTART_FILE" <<DESKTOP_EOF
[Desktop Entry]
Type=Application
Name=IME Cursor Indicator
Exec=$HOME/.local/bin/ime-cursor-indicator
Hidden=false
X-GNOME-Autostart-enabled=true
X-GNOME-Autostart-Delay=4
OnlyShowIn=GNOME;Unity;XFCE;MATE;Cinnamon;
Terminal=false
DESKTOP_EOF
  desktop_status="overwrote autostart desktop entry"

  log "Installation summary:"
  printf '  - Binary: %s\n' "$binary_status"
  printf '  - Config directory: ensured %s\n' "$TARGET_CONFIG_DIR"
  printf '  - Config: %s\n' "$config_status"
  printf '  - Autostart: %s\n' "$desktop_status"
  printf '  - Binary path: %s\n' "$TARGET_BINARY"
  printf '  - Config path: %s\n' "$TARGET_CONFIG"
  printf '  - Desktop entry: %s\n' "$AUTOSTART_FILE"
}

main "$@"
