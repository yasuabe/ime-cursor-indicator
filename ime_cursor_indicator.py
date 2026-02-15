#!/usr/bin/env python3
"""IME mode indicator near mouse cursor for Ubuntu/X11 + IBus.

Prototype implementation based on DESIGN.md.
"""

from __future__ import annotations

import argparse
import signal
from dataclasses import dataclass

import gi

gi.require_version("IBus", "1.0")
from gi.repository import GLib, IBus
from Xlib import X, Xatom, display
from Xlib.error import BadName


@dataclass
class IndicatorState:
    engine_name: str = ""
    label: str = "A"


class IBusWatcher:
    def __init__(self, bus: IBus.Bus, on_label_changed):
        self.bus = bus
        self.on_label_changed = on_label_changed
        self.state = IndicatorState()
        self.current_context_path = ""
        self.current_ic = None
        self.ic_handler_id = None

        self.bus.set_watch_ibus_signal(True)
        self.bus.set_watch_dbus_signal(True)
        self.bus.connect("global-engine-changed", self._on_global_engine_changed)

    def initialize(self):
        self._refresh_engine()
        self._refresh_current_context()

    def _refresh_engine(self):
        engine = self.bus.get_global_engine()
        if engine is not None:
            self._update_engine(engine.get_name())

    def _refresh_current_context(self):
        path = self.bus.current_input_context()
        if path:
            self._watch_input_context(path)

    def tick(self):
        # Focus can move across contexts without engine changes.
        self._refresh_current_context()
        return True

    def _on_global_engine_changed(self, _bus, engine_name):
        if hasattr(engine_name, "get_name"):
            engine_name = engine_name.get_name()
        self._update_engine(engine_name)
        self._refresh_current_context()

    def _watch_input_context(self, context_path: str):
        if context_path == self.current_context_path:
            return
        # Disconnect previous IC signal
        if self.current_ic is not None and self.ic_handler_id is not None:
            self.current_ic.disconnect(self.ic_handler_id)
            self.ic_handler_id = None
        conn = self.bus.get_connection()
        ic = IBus.InputContext.get_input_context(context_path, conn)
        if ic is None:
            # Clear stale path so next tick retries binding this context.
            self.current_context_path = ""
            return
        self.current_ic = ic
        self.current_context_path = context_path
        self.ic_handler_id = self.current_ic.connect(
            "update-property", self._on_property_updated
        )

    def _on_property_updated(self, _ic, prop):
        key = prop.get_key() or ""
        symbol_obj = prop.get_symbol()
        symbol = symbol_obj.get_text() if symbol_obj else ""

        if "inputmode" not in key.lower():
            return

        label = self._label_from_symbol(symbol)
        if label == self.state.label:
            return
        self.state.label = label
        self.on_label_changed(label)

    @staticmethod
    def _label_from_symbol(symbol: str) -> str:
        if symbol in ("あ", "ア", "ｱ"):
            return "あ"
        if symbol in ("A", "_"):
            return "A"
        # Fallback heuristics
        if symbol:
            lowered = symbol.lower()
            if any(k in lowered for k in ("hiragana", "katakana")):
                return "あ"
            if any(k in lowered for k in ("latin", "direct", "alphanumeric")):
                return "A"
        return "A"

    def _update_engine(self, engine_name: str):
        if not engine_name:
            return
        label = self._label_from_engine(engine_name)
        changed = label != self.state.label or engine_name != self.state.engine_name
        self.state.engine_name = engine_name
        self.state.label = label
        if changed:
            self.on_label_changed(label)

    @staticmethod
    def _label_from_engine(engine_name: str) -> str:
        lowered = engine_name.lower()
        if any(k in lowered for k in ("mozc", "anthy", "kkc", "japanese", "kana")):
            return "あ"
        return "A"

    def close(self):
        if self.current_ic is not None and self.ic_handler_id is not None:
            self.current_ic.disconnect(self.ic_handler_id)
            self.ic_handler_id = None


class OverlayWindow:
    def __init__(
        self,
        x_display,
        *,
        poll_ms: int,
        offset_x: int,
        offset_y: int,
        width: int,
        height: int,
        opacity: float,
    ):
        self.display = x_display
        self.screen = self.display.screen()
        self.root = self.screen.root
        self.poll_ms = poll_ms
        self.offset_x = offset_x
        self.offset_y = offset_y
        self.width = width
        self.height = height
        self.opacity = max(0.1, min(1.0, opacity))
        self.label = "A"

        self.window = self.root.create_window(
            0,
            0,
            self.width,
            self.height,
            0,
            self.screen.root_depth,
            X.InputOutput,
            X.CopyFromParent,
            background_pixel=self.screen.black_pixel,
            border_pixel=self.screen.black_pixel,
            override_redirect=True,
            event_mask=(X.ExposureMask),
        )

        self._set_window_hints()
        self._set_opacity(self.opacity)

        self.font = self._load_font(
            [
                "-misc-fixed-bold-r-normal--20-*-*-*-*-*-iso10646-1",
                "-misc-fixed-medium-r-normal--20-*-*-*-*-*-iso10646-1",
                "fixed",
            ]
        )
        self.gc = self.window.create_gc(
            foreground=self.screen.white_pixel,
            background=self.screen.black_pixel,
            font=self.font,
        )

        self.window.map()
        self.display.flush()
        self.redraw()

    def _set_window_hints(self):
        wm_type = self.display.intern_atom("_NET_WM_WINDOW_TYPE")
        wm_type_notification = self.display.intern_atom("_NET_WM_WINDOW_TYPE_NOTIFICATION")
        wm_state = self.display.intern_atom("_NET_WM_STATE")
        wm_state_above = self.display.intern_atom("_NET_WM_STATE_ABOVE")
        wm_state_sticky = self.display.intern_atom("_NET_WM_STATE_STICKY")
        wm_state_skip_taskbar = self.display.intern_atom("_NET_WM_STATE_SKIP_TASKBAR")
        wm_state_skip_pager = self.display.intern_atom("_NET_WM_STATE_SKIP_PAGER")

        self.window.change_property(wm_type, atom_type("ATOM"), 32, [wm_type_notification])
        self.window.change_property(
            wm_state,
            atom_type("ATOM"),
            32,
            [wm_state_above, wm_state_sticky, wm_state_skip_taskbar, wm_state_skip_pager],
        )

    def _set_opacity(self, opacity: float):
        atom = self.display.intern_atom("_NET_WM_WINDOW_OPACITY")
        value = int(0xFFFFFFFF * opacity)
        self.window.change_property(atom, atom_type("CARDINAL"), 32, [value])

    def _load_font(self, names):
        for name in names:
            try:
                return self.display.open_font(name)
            except BadName:
                continue
        return self.display.open_font("fixed")

    def set_label(self, label: str):
        if label == self.label:
            return
        self.label = label
        self.redraw()

    def redraw(self):
        self.window.clear_area()
        # Keep text centered enough for compact labels like "A" / "あ".
        text_x = max(4, self.width // 3)
        text_y = int(self.height * 0.72)
        self.window.draw_text(self.gc, text_x, text_y, self.label)
        self.display.flush()

    def tick(self):
        pointer = self.root.query_pointer()
        x = int(pointer.root_x + self.offset_x)
        y = int(pointer.root_y + self.offset_y)
        x = max(0, min(x, self.screen.width_in_pixels - self.width))
        y = max(0, min(y, self.screen.height_in_pixels - self.height))
        self.window.configure(x=x, y=y)
        self.display.flush()
        return True

    def close(self):
        self.window.unmap()
        self.window.destroy()
        self.display.flush()

def atom_type(name: str) -> int:
    if name == "ATOM":
        return Xatom.ATOM
    if name == "CARDINAL":
        return Xatom.CARDINAL
    raise ValueError(f"unsupported atom type: {name}")


def parse_args():
    parser = argparse.ArgumentParser(
        description="Show IME status indicator near cursor on Ubuntu/X11 + IBus"
    )
    parser.add_argument("--poll-ms", type=int, default=75, help="cursor polling interval")
    parser.add_argument("--offset-x", type=int, default=20, help="x offset from cursor")
    parser.add_argument("--offset-y", type=int, default=18, help="y offset from cursor")
    parser.add_argument("--width", type=int, default=34)
    parser.add_argument("--height", type=int, default=34)
    parser.add_argument("--opacity", type=float, default=0.70)
    return parser.parse_args()


def main():
    args = parse_args()

    IBus.init()

    x_display = display.Display()
    overlay = OverlayWindow(
        x_display,
        poll_ms=args.poll_ms,
        offset_x=args.offset_x,
        offset_y=args.offset_y,
        width=args.width,
        height=args.height,
        opacity=args.opacity,
    )

    ibus_bus = IBus.Bus()
    watcher = IBusWatcher(ibus_bus, overlay.set_label)
    watcher.initialize()

    loop = GLib.MainLoop()

    def _shutdown(*_args):
        watcher.close()
        overlay.close()
        loop.quit()

    signal.signal(signal.SIGINT, _shutdown)
    signal.signal(signal.SIGTERM, _shutdown)

    GLib.timeout_add(args.poll_ms, overlay.tick)
    GLib.timeout_add(500, watcher.tick)
    loop.run()


if __name__ == "__main__":
    main()
