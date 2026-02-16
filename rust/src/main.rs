use std::cell::RefCell;
use std::f64::consts::PI;
use std::fs;
use std::rc::Rc;

use gdk::prelude::*;
use gtk::prelude::*;

struct OverlayState {
    label: String,
    width: i32,
    height: i32,
    opacity: f64,
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

fn update_label(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window, label: &str) {
    {
        let mut st = state.borrow_mut();
        if st.label == label {
            return;
        }
        st.label = label.to_string();
    }
    if label == "A" {
        window.hide();
    } else {
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

    // Add eavesdrop match rule for UpdateProperty
    let match_rule = glib::Variant::tuple_from_iter([
        "eavesdrop=true,type='signal',interface='org.freedesktop.IBus.InputContext',member='UpdateProperty'"
            .to_variant(),
    ]);
    let _ = conn.call_sync(
        Some("org.freedesktop.DBus"),
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "AddMatch",
        Some(&match_rule),
        None,
        gio::DBusCallFlags::NONE,
        -1,
        None::<&gio::Cancellable>,
    );

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
