use std::cell::RefCell;
use std::f64::consts::PI;
use std::fs;
use std::rc::Rc;
use std::time::Duration;

use gdk::prelude::*;
use gtk::prelude::*;

struct OverlayState {
    label: String,
    width: i32,
    height: i32,
    opacity: f64,
    caret_x: i32,
    caret_y: i32,
    caret_h: i32,
    caret_known: bool,
    offset_x: i32,
    offset_y: i32,
    poll_ms: u64,
    pointer_poll_source: Option<glib::SourceId>,
    last_window_x: i32,
    last_window_y: i32,
}

fn rounded_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    ctx.arc(x + r, y + h - r, r, PI / 2.0, PI);
    ctx.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    ctx.close_path();
}

fn label_from_engine(engine_name: &str) -> &'static str {
    let lower = engine_name.to_lowercase();
    for keyword in &["mozc", "anthy", "kkc", "japanese", "kana"] {
        if lower.contains(keyword) {
            return "\u{3042}"; // あ
        }
    }
    "A"
}

fn label_from_symbol(symbol: &str) -> &'static str {
    match symbol {
        "\u{3042}" | "\u{30A2}" | "\u{FF71}" => "\u{3042}", // あ / ア / ｱ
        "A" | "_" => "A",
        _ => {
            let lower = symbol.to_lowercase();
            if lower.contains("hiragana") || lower.contains("katakana") {
                "\u{3042}"
            } else if lower.contains("latin")
                || lower.contains("direct")
                || lower.contains("alphanumeric")
            {
                "A"
            } else {
                "A"
            }
        }
    }
}

fn discover_ibus_address() -> Option<String> {
    if let Ok(addr) = std::env::var("IBUS_ADDRESS") {
        if !addr.is_empty() {
            return Some(addr);
        }
    }

    let machine_id = fs::read_to_string("/var/lib/dbus/machine-id")
        .or_else(|_| fs::read_to_string("/etc/machine-id"))
        .ok()
        .map(|s| s.trim().to_string())?;

    let display = std::env::var("DISPLAY").unwrap_or_else(|_| ":0".to_string());
    let display_num = display
        .trim_start_matches(':')
        .split('.')
        .next()
        .unwrap_or("0");

    let ibus_dir = format!(
        "{}/.config/ibus/bus",
        std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
    );

    let target_suffix = format!("{}-unix-{}", machine_id, display_num);
    let mut socket_file = None;
    if let Ok(entries) = fs::read_dir(&ibus_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.contains(&target_suffix) {
                socket_file = Some(entry.path());
                break;
            }
        }
    }

    let content = fs::read_to_string(socket_file?).ok()?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(addr) = line.strip_prefix("IBUS_ADDRESS=") {
            return Some(addr.to_string());
        }
    }
    None
}

/// Extract engine name from GetGlobalEngine reply.
///
/// Reply is (v) containing IBusEngineDesc tuple; field[2] is the engine name.
fn extract_engine_name(reply: &glib::Variant) -> Option<String> {
    // reply: (v), child 0 is variant, unwrap variant via child_value(0)
    let variant = reply.child_value(0);
    let engine_desc = variant.child_value(0);
    let name = engine_desc.try_child_value(2)?;
    Some(name.str()?.to_string())
}

/// Extract (key, symbol_text) from UpdateProperty signal params.
///
/// Params is (v) containing IBusProperty tuple.
/// IBusProperty: [0] type_name, [1] props, [2] key, ... [11] symbol (IBusText).
/// IBusText: [0] type_name, [1] props, [2] text.
fn extract_property_key_and_symbol(params: &glib::Variant) -> Option<(String, String)> {
    // params: (v), unwrap variant
    let prop_variant = params.child_value(0);
    let prop = prop_variant.child_value(0);
    let n = prop.n_children();
    if n < 12 {
        return None;
    }
    let key = prop.try_child_value(2)?.str()?.to_string();
    // field[11] is symbol: variant wrapping IBusText struct
    let symbol_variant = prop.try_child_value(11)?;
    let symbol_text = symbol_variant.child_value(0);
    if symbol_text.n_children() < 3 {
        return None;
    }
    let symbol = symbol_text.try_child_value(2)?.str()?.to_string();
    Some((key, symbol))
}

fn clamp_to_monitor(
    display: &gdk::Display,
    anchor_x: i32,
    anchor_y: i32,
    target_x: i32,
    target_y: i32,
    width: i32,
    height: i32,
) -> (i32, i32) {
    if let Some(monitor) = display
        .monitor_at_point(anchor_x, anchor_y)
        .or_else(|| display.primary_monitor())
        .or_else(|| display.monitor(0))
    {
        let geom = monitor.geometry();
        let x = target_x.clamp(geom.x(), geom.x() + geom.width() - width);
        let y = target_y.clamp(geom.y(), geom.y() + geom.height() - height);
        (x, y)
    } else {
        (target_x, target_y)
    }
}

fn move_overlay_if_changed(
    state: &Rc<RefCell<OverlayState>>,
    window: &gtk::Window,
    anchor_x: i32,
    anchor_y: i32,
    target_x: i32,
    target_y: i32,
) {
    let (next_x, next_y) = if let Some(display) = gdk::Display::default() {
        let st = state.borrow();
        clamp_to_monitor(&display, anchor_x, anchor_y, target_x, target_y, st.width, st.height)
    } else {
        (target_x, target_y)
    };

    let mut st = state.borrow_mut();
    if st.last_window_x == next_x && st.last_window_y == next_y {
        return;
    }
    st.last_window_x = next_x;
    st.last_window_y = next_y;
    drop(st);
    window.move_(next_x, next_y);
}

fn move_to_pointer(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window) {
    let (offset_x, offset_y) = {
        let st = state.borrow();
        (st.offset_x, st.offset_y)
    };
    let Some(display) = gdk::Display::default() else {
        return;
    };
    let Some(seat) = display.default_seat() else {
        return;
    };
    let Some(pointer) = seat.pointer() else {
        return;
    };

    let (_, px, py) = pointer.position();
    move_overlay_if_changed(state, window, px, py, px + offset_x, py + offset_y);
}

fn stop_pointer_poll(state: &Rc<RefCell<OverlayState>>) {
    let source = state.borrow_mut().pointer_poll_source.take();
    if let Some(source) = source {
        source.remove();
    }
}

fn start_pointer_poll(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window) {
    let (should_start, poll_ms) = {
        let st = state.borrow();
        (
            st.pointer_poll_source.is_none() && !st.caret_known && st.label != "A",
            st.poll_ms.max(1),
        )
    };
    if !should_start {
        return;
    }

    let state_poll = Rc::clone(state);
    let window_poll = window.clone();
    let source = glib::timeout_add_local(Duration::from_millis(poll_ms), move || {
        let should_continue = {
            let st = state_poll.borrow();
            !st.caret_known && st.label != "A"
        };
        if !should_continue {
            state_poll.borrow_mut().pointer_poll_source = None;
            return glib::ControlFlow::Break;
        }
        move_to_pointer(&state_poll, &window_poll);
        glib::ControlFlow::Continue
    });
    state.borrow_mut().pointer_poll_source = Some(source);
}

fn mark_caret_unknown(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window) {
    {
        let mut st = state.borrow_mut();
        st.caret_known = false;
    }
    if state.borrow().label != "A" {
        move_to_pointer(state, window);
        start_pointer_poll(state, window);
        if !window.is_visible() {
            window.show_all();
        }
    }
}

fn move_to_caret(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window, x: i32, y: i32, h: i32) {
    let (offset_x, offset_y) = {
        let mut st = state.borrow_mut();
        st.caret_x = x;
        st.caret_y = y;
        st.caret_h = h;
        st.caret_known = true;
        (st.offset_x, st.offset_y)
    };
    stop_pointer_poll(state);
    move_overlay_if_changed(state, window, x, y, x + offset_x, y + h + offset_y);
    if state.borrow().label != "A" && !window.is_visible() {
        window.show_all();
    }
}

fn add_match_rule(conn: &gio::DBusConnection, rule: &str) {
    let match_rule = glib::Variant::tuple_from_iter([rule.to_variant()]);
    if let Err(err) = conn.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "AddMatch",
        Some(&match_rule),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        None::<&gio::Cancellable>,
    ) {
        eprintln!("Warning: AddMatch failed for {}: {}", rule, err);
    }
}

fn update_label(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window, label: &str) {
    {
        let mut st = state.borrow_mut();
        if st.label == label {
            return;
        }
        st.label = label.to_string();
    }
    if label == "A" {
        stop_pointer_poll(state);
        window.hide();
    } else {
        if state.borrow().caret_known {
            let (x, y, h) = {
                let st = state.borrow();
                (st.caret_x, st.caret_y, st.caret_h)
            };
            move_to_caret(state, window, x, y, h);
        } else {
            move_to_pointer(state, window);
            start_pointer_poll(state, window);
        }
        window.show_all();
        window.queue_draw();
    }
}

fn setup_ibus(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window) {
    let addr = match discover_ibus_address() {
        Some(a) => a,
        None => {
            eprintln!("Failed to discover IBus address");
            return;
        }
    };

    let conn = match gio::DBusConnection::for_address_sync(
        &addr,
        gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
            | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
        None::<&gio::DBusAuthObserver>,
        None::<&gio::Cancellable>,
    ) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to IBus: {}", e);
            return;
        }
    };

    // Get initial engine
    if let Ok(reply) = conn.call_sync(
        Some("org.freedesktop.IBus"),
        "/org/freedesktop/IBus",
        "org.freedesktop.IBus",
        "GetGlobalEngine",
        None,
        None,
        gio::DBusCallFlags::NONE,
        -1,
        None::<&gio::Cancellable>,
    ) {
        if let Some(name) = extract_engine_name(&reply) {
            let label = label_from_engine(&name);
            update_label(state, window, label);
        }
    }

    // Add eavesdrop match rules for InputContext signals/method calls.
    add_match_rule(
        &conn,
        "eavesdrop=true,type='signal',interface='org.freedesktop.IBus.InputContext',member='UpdateProperty'",
    );
    add_match_rule(
        &conn,
        "eavesdrop=true,type='method_call',interface='org.freedesktop.IBus.InputContext',member='SetCursorLocation'",
    );
    for member in ["FocusIn", "FocusOut"] {
        add_match_rule(
            &conn,
            &format!(
                "eavesdrop=true,type='method_call',interface='org.freedesktop.IBus.InputContext',member='{}'",
                member
            ),
        );
        add_match_rule(
            &conn,
            &format!(
                "eavesdrop=true,type='signal',interface='org.freedesktop.IBus.InputContext',member='{}'",
                member
            ),
        );
    }

    // Subscribe to GlobalEngineChanged
    let state_eng = Rc::clone(state);
    let window_eng = window.clone();
    conn.signal_subscribe(
        None::<&str>,
        Some("org.freedesktop.IBus"),
        Some("GlobalEngineChanged"),
        Some("/org/freedesktop/IBus"),
        None::<&str>,
        gio::DBusSignalFlags::NONE,
        move |_conn, _sender, _path, _iface, _signal, params| {
            if let Some(name) = params.child_value(0).str() {
                let label = label_from_engine(&name);
                update_label(&state_eng, &window_eng, label);
            }
        },
    );

    // Subscribe to UpdateProperty (eavesdrop — match rule already added)
    let state_prop = Rc::clone(state);
    let window_prop = window.clone();
    conn.signal_subscribe(
        None::<&str>,
        Some("org.freedesktop.IBus.InputContext"),
        Some("UpdateProperty"),
        None::<&str>,
        None::<&str>,
        gio::DBusSignalFlags::NO_MATCH_RULE,
        move |_conn, _sender, _path, _iface, _signal, params| {
            if let Some((key, symbol)) = extract_property_key_and_symbol(params) {
                if key.to_lowercase().contains("inputmode") {
                    let label = label_from_symbol(&symbol);
                    update_label(&state_prop, &window_prop, label);
                }
            }
        },
    );

    let (caret_sender, caret_receiver) =
        glib::MainContext::channel::<(i32, i32, i32, i32)>(glib::Priority::DEFAULT);
    let state_caret_rx = Rc::clone(state);
    let window_caret_rx = window.clone();
    caret_receiver.attach(None, move |(x, y, w, h)| {
        if x == 0 && y == 0 && w == 0 && h == 0 {
            mark_caret_unknown(&state_caret_rx, &window_caret_rx);
        } else {
            move_to_caret(&state_caret_rx, &window_caret_rx, x, y, h);
        }
        glib::ControlFlow::Continue
    });

    // Track active context path to ignore noise from unfocused InputContexts.
    let focused_ic_path = Rc::new(RefCell::new(None::<String>));
    let last_cursor = Rc::new(RefCell::new((-1, -1, -1, -1)));

    // Intercept InputContext method calls via filter (eavesdrop rules already added)
    let caret_sender_filter = caret_sender.clone();
    let focused_ic_path_filter = Rc::clone(&focused_ic_path);
    let last_cursor_filter = Rc::clone(&last_cursor);
    let _caret_filter_id = conn.add_filter(move |_conn, message, incoming| {
        if !incoming {
            return Some(message.clone());
        }
        if message.interface().as_deref() != Some("org.freedesktop.IBus.InputContext") {
            return Some(message.clone());
        }
        let msg_type = message.message_type();
        let is_method_or_signal =
            msg_type == gio::DBusMessageType::MethodCall || msg_type == gio::DBusMessageType::Signal;
        if !is_method_or_signal {
            return Some(message.clone());
        }

        let path = message.path().map(|p| p.to_string());
        let member = message.member();
        let member = member.as_deref();
        if member == Some("FocusIn") {
            *focused_ic_path_filter.borrow_mut() = path;
            return Some(message.clone());
        }
        if member == Some("FocusOut") {
            if focused_ic_path_filter.borrow().as_deref() == path.as_deref() {
                *focused_ic_path_filter.borrow_mut() = None;
                *last_cursor_filter.borrow_mut() = (-1, -1, -1, -1);
                let _ = caret_sender_filter.send((0, 0, 0, 0));
            }
            return Some(message.clone());
        }
        if member != Some("SetCursorLocation") {
            return Some(message.clone());
        }
        if msg_type != gio::DBusMessageType::MethodCall {
            return Some(message.clone());
        }

        if focused_ic_path_filter.borrow().is_none() {
            *focused_ic_path_filter.borrow_mut() = path.clone();
        }
        if focused_ic_path_filter.borrow().as_deref() != path.as_deref() {
            return Some(message.clone());
        }

        let Some(body) = message.body() else {
            return Some(message.clone());
        };
        if body.n_children() < 4 {
            return Some(message.clone());
        }

        let x = body.child_value(0).get::<i32>();
        let y = body.child_value(1).get::<i32>();
        let w = body.child_value(2).get::<i32>();
        let h = body.child_value(3).get::<i32>();
        let (Some(x), Some(y), Some(w), Some(h)) = (x, y, w, h) else {
            return Some(message.clone());
        };

        if x == 0 && y == 0 && w == 0 && h == 0 {
            *last_cursor_filter.borrow_mut() = (-1, -1, -1, -1);
            let _ = caret_sender_filter.send((0, 0, 0, 0));
            return Some(message.clone());
        }

        let coords = (x, y, w, h);
        if *last_cursor_filter.borrow() == coords {
            return Some(message.clone());
        }
        *last_cursor_filter.borrow_mut() = coords;

        let _ = caret_sender_filter.send((x, y, w, h));

        Some(message.clone())
    });

    // Keep the connection alive for the lifetime of the program.
    // Leaking is intentional — the connection must outlive signal subscriptions.
    std::mem::forget(conn);
}

fn main() {
    gtk::init().expect("Failed to initialize GTK");

    let state = Rc::new(RefCell::new(OverlayState {
        label: "A".to_string(),
        width: 34,
        height: 34,
        opacity: 0.7,
        caret_x: 0,
        caret_y: 0,
        caret_h: 0,
        caret_known: false,
        offset_x: 20,
        offset_y: 18,
        poll_ms: 75,
        pointer_poll_source: None,
        last_window_x: i32::MIN,
        last_window_y: i32::MIN,
    }));

    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_app_paintable(true);
    window.set_decorated(false);
    window.set_keep_above(true);
    window.stick();
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_type_hint(gdk::WindowTypeHint::Notification);

    {
        let st = state.borrow();
        window.set_default_size(st.width, st.height);
    }

    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    let state_draw = Rc::clone(&state);
    window.connect_draw(move |_widget, ctx| {
        let st = state_draw.borrow();
        let w = st.width as f64;
        let h = st.height as f64;

        ctx.set_operator(cairo::Operator::Source);
        ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        ctx.paint().expect("paint failed");
        ctx.set_operator(cairo::Operator::Over);

        let r = w.min(h) * 0.32;
        rounded_rect(ctx, 0.0, 0.0, w, h, r);

        if st.label == "\u{3042}" {
            ctx.set_source_rgba(0.8, 0.0, 0.0, st.opacity);
        } else {
            ctx.set_source_rgba(0.0, 0.0, 0.0, st.opacity);
        }
        ctx.fill().expect("fill failed");

        let layout = pangocairo::functions::create_layout(ctx);
        let font_desc = pango::FontDescription::from_string("Sans Bold 16");
        layout.set_font_description(Some(&font_desc));
        layout.set_text(&st.label);
        let (_, logical) = layout.pixel_extents();
        let tx = (w as i32 - logical.width()) / 2 - logical.x();
        let ty = (h as i32 - logical.height()) / 2 - logical.y();
        ctx.move_to(tx as f64, ty as f64);
        ctx.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        pangocairo::functions::show_layout(ctx, &layout);

        glib::Propagation::Stop
    });

    if let Some(display) = gdk::Display::default() {
        if let Some(monitor) = display.primary_monitor().or_else(|| display.monitor(0)) {
            let geom = monitor.geometry();
            let st = state.borrow();
            let x = geom.x() + (geom.width() - st.width) / 2;
            let y = geom.y() + (geom.height() - st.height) / 2;
            window.move_(x, y);
        }
    }

    window.show_all();

    setup_ibus(&state, &window);

    glib::unix_signal_add_local(libc::SIGINT, || {
        gtk::main_quit();
        glib::ControlFlow::Break
    });

    gtk::main();
}
