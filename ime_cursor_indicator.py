#!/usr/bin/env python3
"""IME mode indicator near text caret for Ubuntu/X11 + IBus.

Tracks the caret position by eavesdropping on IBus SetCursorLocation
method calls on the IBus private bus.
"""

from __future__ import annotations

import argparse
import os
import shutil
import signal
import sys
import tempfile
import tomllib
from dataclasses import dataclass
from pathlib import Path

import math

import cairo
import gi

gi.require_version("Gdk", "3.0")
gi.require_version("Gtk", "3.0")
gi.require_version("IBus", "1.0")
gi.require_version("Gio", "2.0")
gi.require_version("Pango", "1.0")
gi.require_version("PangoCairo", "1.0")
gi.require_version("AyatanaAppIndicator3", "0.1")
from gi.repository import Gdk, Gio, GLib, Gtk, IBus, Pango, PangoCairo
from gi.repository import AyatanaAppIndicator3 as AppIndicator3


@dataclass
class OverlayStyle:
    offset_x: int
    offset_y: int
    width: int
    height: int
    opacity: float


@dataclass
class IndicatorState:
    engine_name: str = ""
    label: str = "A"


class CaretTracker:
    """Eavesdrop on IBus SetCursorLocation to track the text caret position."""

    _IC_IFACE = "org.freedesktop.IBus.InputContext"

    def __init__(self, bus: IBus.Bus, on_cursor_moved, on_cursor_lost):
        self._bus = bus
        self._on_cursor_moved = on_cursor_moved
        self._on_cursor_lost = on_cursor_lost
        self._last = (-1, -1, -1, -1)

    def start(self):
        conn = self._bus.get_connection()

        for member in ("SetCursorLocation", "FocusOut"):
            match_rule = (
                "eavesdrop=true,"
                "type='method_call',"
                f"interface='{self._IC_IFACE}',"
                f"member='{member}'"
            )
            try:
                conn.call_sync(
                    "org.freedesktop.DBus",
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "AddMatch",
                    GLib.Variant("(s)", (match_rule,)),
                    None,
                    Gio.DBusCallFlags.NONE,
                    -1,
                    None,
                )
            except Exception as exc:
                print(f"Warning: AddMatch for {member} failed: {exc}",
                      file=sys.stderr)
                return
        conn.add_filter(self._message_filter)

    def _message_filter(self, connection, message, incoming):
        if not incoming:
            return message
        if message.get_interface() != self._IC_IFACE:
            return message

        member = message.get_member()

        if member == "FocusOut":
            self._last = (-1, -1, -1, -1)
            GLib.idle_add(self._on_cursor_lost)
            return message

        if member != "SetCursorLocation":
            return message

        body = message.get_body()
        if body is None or body.n_children() < 4:
            return message

        x = body.get_child_value(0).get_int32()
        y = body.get_child_value(1).get_int32()
        w = body.get_child_value(2).get_int32()
        h = body.get_child_value(3).get_int32()

        # (0,0,0,0) is a focus-loss reset
        if x == 0 and y == 0 and w == 0 and h == 0:
            self._last = (-1, -1, -1, -1)
            GLib.idle_add(self._on_cursor_lost)
            return message

        # Deduplicate
        coords = (x, y, w, h)
        if coords == self._last:
            return message
        self._last = coords

        GLib.idle_add(self._on_cursor_moved, x, y, w, h)
        return message


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
            # Clear stale state so next tick retries binding this context.
            self.current_context_path = ""
            self.current_ic = None
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
        *,
        on_style: OverlayStyle,
        off_style: OverlayStyle,
    ):
        self.on_style = on_style
        self.off_style = off_style
        self.label = "A"
        self._visible = False
        self._caret_known = False

        # Start with off_style
        style = self.off_style
        self.offset_x = style.offset_x
        self.offset_y = style.offset_y
        self.width = style.width
        self.height = style.height
        self.opacity = max(0.1, min(1.0, style.opacity))

        self.font_desc = Pango.FontDescription("Sans Bold 16")

        self.window = Gtk.Window(type=Gtk.WindowType.POPUP)
        self.window.set_app_paintable(True)
        self.window.set_decorated(False)
        self.window.set_keep_above(True)
        self.window.stick()
        self.window.set_skip_taskbar_hint(True)
        self.window.set_skip_pager_hint(True)
        self.window.set_type_hint(Gdk.WindowTypeHint.NOTIFICATION)
        self.window.set_default_size(self.width, self.height)

        # Enable per-pixel alpha via RGBA visual
        screen = self.window.get_screen()
        visual = screen.get_rgba_visual()
        if visual:
            self.window.set_visual(visual)

        self.window.connect("draw", self._on_draw)
        # Start hidden; shown when IME is on and caret position is known
        self.window.realize()

    def _on_draw(self, widget, ctx):
        # Clear to fully transparent
        ctx.set_operator(cairo.OPERATOR_SOURCE)
        ctx.set_source_rgba(0, 0, 0, 0)
        ctx.paint()
        ctx.set_operator(cairo.OPERATOR_OVER)

        # Draw rounded rectangle background
        r = min(self.width, self.height) * 0.32
        self._rounded_rect(ctx, 0, 0, self.width, self.height, r)
        if self.label == "あ":
            ctx.set_source_rgba(0.8, 0, 0, self.opacity)
        else:
            ctx.set_source_rgba(0, 0, 0, self.opacity)
        ctx.fill()

        # Draw centered text
        layout = PangoCairo.create_layout(ctx)
        layout.set_font_description(self.font_desc)
        layout.set_text(self.label, -1)
        _ink, logical = layout.get_pixel_extents()
        tx = (self.width - logical.width) // 2 - logical.x
        ty = (self.height - logical.height) // 2 - logical.y
        ctx.move_to(tx, ty)
        ctx.set_source_rgba(1, 1, 1, 1)
        PangoCairo.show_layout(ctx, layout)
        return True

    @staticmethod
    def _rounded_rect(ctx, x, y, w, h, r):
        ctx.new_sub_path()
        ctx.arc(x + w - r, y + r, r, -math.pi / 2, 0)
        ctx.arc(x + w - r, y + h - r, r, 0, math.pi / 2)
        ctx.arc(x + r, y + h - r, r, math.pi / 2, math.pi)
        ctx.arc(x + r, y + r, r, math.pi, 3 * math.pi / 2)
        ctx.close_path()

    def set_label(self, label: str):
        if label == self.label:
            return
        self.label = label
        style = self.on_style if label == "あ" else self.off_style
        self.offset_x = style.offset_x
        self.offset_y = style.offset_y
        self.opacity = max(0.1, min(1.0, style.opacity))
        if style.width != self.width or style.height != self.height:
            self.width = style.width
            self.height = style.height
            self.window.resize(self.width, self.height)
        self.redraw()

    def redraw(self):
        self.window.queue_draw()

    def move_to_caret(self, cx, cy, cw, ch):
        """Position the overlay relative to the caret rectangle."""
        self._caret_known = True
        display = Gdk.Display.get_default()
        monitor = display.get_monitor_at_point(cx, cy)
        geom = monitor.get_geometry()
        x = cx + self.offset_x
        y = cy + ch + self.offset_y
        x = max(geom.x, min(x, geom.x + geom.width - self.width))
        y = max(geom.y, min(y, geom.y + geom.height - self.height))
        self.window.move(x, y)
        if self._visible and not self.window.get_visible():
            self.window.show_all()

    def _move_to_pointer(self):
        display = Gdk.Display.get_default()
        seat = display.get_default_seat()
        pointer = seat.get_pointer()
        screen, px, py = pointer.get_position()
        monitor = display.get_monitor_at_point(px, py)
        geom = monitor.get_geometry()
        x = px + self.offset_x
        y = py + self.offset_y
        x = max(geom.x, min(x, geom.x + geom.width - self.width))
        y = max(geom.y, min(y, geom.y + geom.height - self.height))
        self.window.move(x, y)

    def show(self):
        self._visible = True
        if not self._caret_known:
            self._move_to_pointer()
        self.window.show_all()

    def hide(self):
        self._visible = False
        self.window.hide()

    def mark_caret_lost(self):
        """Called when SetCursorLocation(0,0,0,0) indicates focus loss."""
        self._caret_known = False
        if self._visible:
            self._move_to_pointer()
            if not self.window.get_visible():
                self.window.show_all()

    def tick_pointer(self):
        """Poll mouse position only when caret position is unknown."""
        if self._visible and not self._caret_known:
            self._move_to_pointer()
        return True

    def close(self):
        self.window.destroy()

class TrayIndicator:
    def __init__(self, on_quit):
        self._icon_dir = tempfile.mkdtemp(prefix="ime-indicator-")
        self._icon_a = self._create_icon((0, 0, 0), "A", "icon_a.png")
        self._icon_ja = self._create_icon((0.8, 0, 0), "あ", "icon_ja.png")

        self.indicator = AppIndicator3.Indicator.new(
            "ime-cursor-indicator",
            "icon_a",
            AppIndicator3.IndicatorCategory.APPLICATION_STATUS,
        )
        self.indicator.set_icon_theme_path(self._icon_dir)
        self.indicator.set_status(AppIndicator3.IndicatorStatus.ACTIVE)
        self.indicator.set_label("", "")

        menu = Gtk.Menu()
        item_quit = Gtk.MenuItem(label="Quit")
        item_quit.connect("activate", lambda _: on_quit())
        menu.append(item_quit)
        menu.show_all()
        self.indicator.set_menu(menu)

    def _create_icon(self, rgb, label, filename):
        path = os.path.join(self._icon_dir, filename)
        size = 22
        surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
        ctx = cairo.Context(surface)
        # Background circle
        ctx.set_source_rgb(*rgb)
        ctx.arc(size / 2, size / 2, size / 2, 0, 2 * 3.14159)
        ctx.fill()
        # White label text
        layout = PangoCairo.create_layout(ctx)
        layout.set_font_description(Pango.FontDescription("Sans Bold 14"))
        layout.set_text(label, -1)
        _ink, logical = layout.get_pixel_extents()
        tx = (size - logical.width) / 2 - logical.x
        ty = (size - logical.height) / 2 - logical.y
        ctx.move_to(tx, ty)
        ctx.set_source_rgb(1, 1, 1)
        PangoCairo.show_layout(ctx, layout)
        surface.write_to_png(path)
        return os.path.splitext(filename)[0]

    def set_label(self, label: str):
        icon = self._icon_ja if label == "あ" else self._icon_a
        self.indicator.set_icon_full(icon, label)

    def close(self):
        shutil.rmtree(self._icon_dir, ignore_errors=True)


_CONFIG_SCHEMA: dict[str, tuple[type, object]] = {
    "poll_ms": (int, 75),
    "offset_x": (int, 20),
    "offset_y": (int, 18),
    "width": (int, 34),
    "height": (int, 34),
    "opacity": (float, 0.70),
}


_STYLE_KEYS = {"offset_x", "offset_y", "width", "height", "opacity"}


def _validate_section(raw: dict, section_name: str) -> dict:
    """Validate keys against _CONFIG_SCHEMA types, returning valid entries."""
    result: dict = {}
    for key, (expected_type, default) in _CONFIG_SCHEMA.items():
        if section_name and key not in _STYLE_KEYS:
            continue
        if key not in raw:
            continue
        value = raw[key]
        if isinstance(value, expected_type):
            result[key] = value
        elif expected_type is float and isinstance(value, int):
            result[key] = float(value)
        else:
            label = f"[{section_name}].{key}" if section_name else f"'{key}'"
            print(f"Warning: config {label} should be {expected_type.__name__}, "
                  f"got {type(value).__name__}; using default ({default})")
    return result


def load_config() -> dict:
    path = Path.home() / ".config" / "ime-cursor-indicator" / "config.toml"
    if not path.exists():
        return {}
    try:
        with open(path, "rb") as f:
            raw = tomllib.load(f)
    except Exception as e:
        print(f"Warning: failed to load {path}: {e}", file=sys.stderr)
        return {}
    config = _validate_section(raw, "")
    for section in ("on", "off"):
        if section not in raw:
            continue
        if isinstance(raw[section], dict):
            config[section] = _validate_section(raw[section], section)
        else:
            print(f"Warning: config [{section}] should be a table, "
                  f"got {type(raw[section]).__name__}; ignoring", file=sys.stderr)
    return config


def parse_args():
    config = load_config()
    parser = argparse.ArgumentParser(
        description="Show IME status indicator near cursor on Ubuntu/X11 + IBus"
    )
    parser.add_argument("--poll-ms", type=int, default=config.get("poll_ms", 75), help="cursor polling interval")
    parser.add_argument("--offset-x", type=int, default=config.get("offset_x", 20), help="x offset from cursor")
    parser.add_argument("--offset-y", type=int, default=config.get("offset_y", 18), help="y offset from cursor")
    parser.add_argument("--width", type=int, default=config.get("width", 34))
    parser.add_argument("--height", type=int, default=config.get("height", 34))
    parser.add_argument("--opacity", type=float, default=config.get("opacity", 0.70))
    args = parser.parse_args()

    # Build base style from CLI args (which already incorporate top-level config)
    base = {
        "offset_x": args.offset_x,
        "offset_y": args.offset_y,
        "width": args.width,
        "height": args.height,
        "opacity": args.opacity,
    }
    # Override with [on]/[off] section values
    on_vals = {**base, **config.get("on", {})}
    off_vals = {**base, **config.get("off", {})}
    on_style = OverlayStyle(**on_vals)
    off_style = OverlayStyle(**off_vals)

    return args, on_style, off_style


def main():
    args, on_style, off_style = parse_args()

    IBus.init()

    overlay = OverlayWindow(
        on_style=on_style,
        off_style=off_style,
    )

    loop = GLib.MainLoop()

    def _shutdown(*_args):
        watcher.close()
        overlay.close()
        tray.close()
        loop.quit()

    tray = TrayIndicator(on_quit=_shutdown)

    def on_label_changed(label: str):
        overlay.set_label(label)
        tray.set_label(label)
        if label == "あ":
            overlay.show()
        else:
            overlay.hide()

    ibus_bus = IBus.Bus()
    watcher = IBusWatcher(ibus_bus, on_label_changed)
    watcher.initialize()

    def on_cursor_moved(x, y, w, h):
        overlay.move_to_caret(x, y, w, h)

    def on_cursor_lost():
        overlay.mark_caret_lost()
        return False

    caret_tracker = CaretTracker(ibus_bus, on_cursor_moved, on_cursor_lost)
    caret_tracker.start()

    signal.signal(signal.SIGINT, _shutdown)
    signal.signal(signal.SIGTERM, _shutdown)

    GLib.timeout_add(args.poll_ms, overlay.tick_pointer)
    GLib.timeout_add(500, watcher.tick)
    loop.run()


if __name__ == "__main__":
    main()
