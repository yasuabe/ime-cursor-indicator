# CLAUDE.md

## Language

- User communication: Japanese
- Commit messages, code, comments: English

## Commit Style

- Start with a verb (Add, Fix, Update, ...)
- First line is a concise summary
- Blank line followed by details when needed

## Verification

- Python: Always run `python3 -m py_compile python/ime_cursor_indicator.py` after changes
- Rust: Always run `cargo check` in `rust/` after changes

## TODO.md

- Mark items as `[x]` when the corresponding feature is implemented

## Review Triage

- Medium/High: fix by default
- Low: evaluate cost/benefit before deciding
