# Specification: ime-cursor-indicator

Current behavior specification based on the Python prototype.

## Overview

Mouse cursor (or text caret) near an IME mode indicator for Ubuntu/X11 + IBus.
Displays `A` (alphanumeric) or `あ` (Japanese input) as a small overlay window.

## Target Environment

- OS: Ubuntu (Linux)
- Display server: X11 only (Wayland is out of scope)
- IME framework: IBus
- Primary IME: Mozc (`mozc-jp`)

## Features

### 1. IME Mode Detection

- **Engine change**: Listen to IBus `global-engine-changed` signal.
  - Engines containing `mozc`, `anthy`, `kkc`, `japanese`, `kana` → label `あ`
  - All others → label `A`
- **Mozc input mode change**: Watch `InputContext.update-property` signal.
  - Filter by property key containing `inputmode` (case-insensitive).
  - Symbol `あ` / `ア` / `ｱ` → label `あ`
  - Symbol `A` / `_` → label `A`
  - Fallback heuristics based on symbol text (`hiragana`, `katakana` → `あ`; `latin`, `direct`, `alphanumeric` → `A`)
- **Focus tracking**: Periodically re-check the current input context (500ms interval) to detect focus changes across applications.

### 2. Overlay Window

- GTK3 popup window (`Gtk.WindowType.POPUP`)
- Transparent background with rounded-rectangle badge
- Text rendered with Pango/Cairo (`Sans Bold 16`)
- Window properties:
  - `override_redirect` (popup type)
  - Always on top (`keep_above`)
  - Does not steal input focus
  - Skips taskbar and pager
  - Window type hint: `NOTIFICATION`
  - Per-pixel alpha via RGBA visual

#### Visual Style

- Japanese input ON (`あ`): **red** background (0.8, 0, 0)
- Alphanumeric (`A`): **black** background (0, 0, 0)
- White text in both cases
- Corner radius: `min(width, height) * 0.32`

#### Visibility Rules

- Shown when IME label is `あ` (Japanese input ON)
- Hidden when IME label is `A` (alphanumeric)

### 3. Caret Tracking

- Eavesdrops on IBus private bus `SetCursorLocation` method calls via D-Bus `AddMatch` with `eavesdrop=true`.
- Receives caret rectangle `(x, y, w, h)`.
- Overlay is positioned relative to the caret: `(x + offset_x, y + h + offset_y)`.
- Clamped to monitor bounds.
- Deduplicates identical coordinates.
- `SetCursorLocation(0, 0, 0, 0)` is treated as focus loss.

### 4. Focus Loss / Pointer Fallback

- On `FocusOut` or `SetCursorLocation(0,0,0,0)`:
  - Caret position is marked as unknown.
  - Overlay falls back to following the mouse pointer.
- Mouse pointer position is polled at `poll_ms` interval (default 75ms) only when caret position is unknown.

### 5. System Tray Indicator

- Uses `AyatanaAppIndicator3`.
- Displays a 22x22 circular icon:
  - `A` on black circle (alphanumeric)
  - `あ` on red circle (Japanese input)
- Icons are rendered as PNG files in a temporary directory at startup.
- Provides a "Quit" menu item.
- Temporary icon files are cleaned up on shutdown.

### 6. Configuration

#### Config File

- Path: `~/.config/ime-cursor-indicator/config.toml`
- Format: TOML

#### Parameters

| Key        | Type  | Default | Description                   |
|------------|-------|---------|-------------------------------|
| `poll_ms`  | int   | 75      | Pointer polling interval (ms) |
| `offset_x` | int   | 20      | X offset from caret/cursor    |
| `offset_y` | int   | 18      | Y offset from caret/cursor    |
| `width`    | int   | 34      | Overlay window width (px)     |
| `height`   | int   | 34      | Overlay window height (px)    |
| `opacity`  | float | 0.70    | Background opacity (0.1–1.0)  |

#### Per-Mode Style Overrides

`[on]` and `[off]` sections in the config file can override style keys (`offset_x`, `offset_y`, `width`, `height`, `opacity`) independently for Japanese-ON and alphanumeric-OFF states.

#### Priority

```
[on]/[off] section  >  CLI args / top-level config  >  hardcoded defaults
```

#### CLI Arguments

```
--poll-ms    (int)
--offset-x   (int)
--offset-y   (int)
--width      (int)
--height     (int)
--opacity    (float)
```

### 7. Lifecycle

- Startup: `IBus.init()` → create overlay → create tray → connect IBus watcher → start caret tracker → enter GLib main loop.
- Shutdown: triggered by `SIGINT`, `SIGTERM`, or tray "Quit". Disconnects signals, destroys windows, cleans up temp files, exits main loop.

## Known Limitations

- Mozc mode detection is best-effort (IBus property structure varies).
- X11 only; no Wayland support.
- Single-monitor clamping uses the monitor at the caret/pointer position.
