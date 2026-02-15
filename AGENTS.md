# AGENTS

## Scope
- This repository is a prototype for `ime-cursor-indicator`.
- Prioritize fast iteration and clarity over perfect architecture.

## Environment Assumptions
- Target OS: Ubuntu.
- Display server: X11 only.
- Wayland is out of scope for current implementation.
- IME stack: IBus with Mozc as primary target.

## Core Technical Decisions
- Use `gi.repository.IBus` (not `dbus-python`) for IBus integration.
- Render overlay text with Pango/Cairo to avoid glyph corruption.
- Overlay is an X11 window near mouse cursor.
- Top bar indicator uses `AyatanaAppIndicator3`.

## Current Behavior Contracts
- Label states are `A` (alphanumeric) and `あ` (Japanese input).
- Visual emphasis:
  - Japanese input ON: red-ish background.
  - Alphanumeric input: black background.
- Status updates are driven by:
  - `global-engine-changed`
  - `InputContext` property updates (`update-property`)
  - periodic context re-check (`tick`) to follow focus changes.

## Config Contract
- Config file path:
  - `~/.config/ime-cursor-indicator/config.toml`
- Example file in repo:
  - `config.toml.example`
- Priority:
  - CLI args > config file > hardcoded defaults.

## Known Gaps / Intentional Deferrals
- Mode detection remains best-effort for Mozc internal mode switches.
- The following are intentionally deferred for now:
  - unify invalid-config warnings to stderr everywhere
  - strict bool-vs-int rejection in config type validation
- Reason: prototype stage and possible future rewrite in Rust or Go.

## Operational Notes
- Temporary tray icon files are created at runtime and should be cleaned up on shutdown.
- Use `TODO.md` as the single source of truth for pending work.

