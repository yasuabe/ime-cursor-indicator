# Migration Plan: Python → Rust

## Goals

- Rewrite the Python prototype in Rust for lower resource usage and faster startup.
- Keep the Python version functional during migration as the reference implementation.
- Maintain the same user-facing behavior (see `SPEC.md`).

## Repository Structure

Monorepo with `python/` and `rust/` subdirectories:

```
ime-cursor-indicator/
├── python/                 # Existing Python prototype (frozen)
│   └── ime_cursor_indicator.py
├── rust/                   # Rust implementation
│   ├── Cargo.toml
│   └── src/
│       └── main.rs
├── config.toml.example     # Shared config format
├── SPEC.md                 # Shared specification
├── MIGRATION.md            # This file
├── DESIGN.md
├── README.md
└── ...
```

## Crate Selection

| Concern            | Crate              | Notes                                    |
|--------------------|---------------------|------------------------------------------|
| GTK3 bindings      | `gtk` (gtk3-rs)     | Matches Python version's GTK3 usage      |
| Cairo drawing      | `cairo-rs`          | Transitive via gtk-rs                    |
| Pango text layout  | `pango`, `pangocairo` | Transitive via gtk-rs                  |
| D-Bus              | `zbus`              | Pure Rust, async-friendly                |
| IBus protocol      | `zbus` (manual)     | No dedicated IBus crate; use D-Bus directly |
| Config (TOML)      | `toml`, `serde`     | Deserialize config.toml                  |
| CLI arguments      | `clap`              | Feature-rich, derive-based               |
| Tray indicator     | `libappindicator`   | FFI bindings to libappindicator          |

## Migration Phases

### Phase 1: Minimal Overlay Window

- Create a GTK3 popup window with Cairo/Pango rendering.
- Display a fixed label (`A`) at a fixed screen position.
- Verify: transparent background, rounded rectangle, text rendering.

### Phase 2: IBus Engine Monitoring

- Connect to IBus via D-Bus (`zbus`).
- Listen for `GlobalEngineChanged` signal.
- Toggle label between `A` and `あ` based on engine name.
- Show overlay on `あ`, hide on `A`.

### Phase 3: Mozc Input Mode Detection

- Watch `InputContext.UpdateProperty` for `inputmode` property changes.
- Track the current input context path.
- Implement the same symbol-to-label mapping as the Python version.

### Phase 4: Caret Tracking

- Eavesdrop on `SetCursorLocation` on the IBus private bus.
- Position overlay relative to the caret rectangle.
- Handle `FocusOut` and `(0,0,0,0)` as caret-loss events.
- Fall back to mouse pointer polling when caret is unknown.

### Phase 5: Configuration and CLI

- Parse `~/.config/ime-cursor-indicator/config.toml` with `toml`/`serde`.
- Parse CLI arguments with `clap`.
- Implement the same priority: `[on]/[off]` > CLI/top-level > defaults.

### Phase 6: System Tray

- Integrate `libappindicator` for the tray icon.
- Render 22x22 PNG icons with Cairo at startup.
- Provide "Quit" menu item.

### Phase 7: Polish and Parity Check

- Signal handling (`SIGINT`, `SIGTERM`).
- Compare behavior side-by-side with Python version.
- Update `README.md` with Rust build/run instructions.
- Keep Python version in-tree for now (no archive/removal yet).

## Testing Strategy

- **Side-by-side manual testing**: Run Python and Rust versions simultaneously (with different `offset_x`) to compare behavior visually.
- **Checklist per phase**:
  - [ ] Overlay appears/disappears correctly on mode change
  - [ ] Overlay follows text caret
  - [ ] Overlay follows mouse pointer on focus loss
  - [ ] Tray icon updates on mode change
  - [ ] Config file and CLI args are respected
  - [ ] `[on]`/`[off]` per-mode styles work
  - [ ] Clean shutdown on SIGINT/SIGTERM and tray Quit
  - [ ] Multi-monitor clamping works

## Coexistence Rules

- **Python version is frozen**: No new features. Bug fixes only if critical.
- **Feature additions go to Rust only** from this point forward.
- **Shared artifacts**: `config.toml.example` and `SPEC.md` are the source of truth for both versions.
- **Retirement**: Once Phase 7 is complete and the Rust version is stable, move `python/` to `legacy/` or remove it.
