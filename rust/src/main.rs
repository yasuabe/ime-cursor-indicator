use std::cell::RefCell;
use std::f64::consts::PI;
use std::fs;
use std::rc::Rc;

use gdk::prelude::*;
use gtk::prelude::*;
use zbus::zvariant;

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

fn discover_ibus_address() -> Result<String, Box<dyn std::error::Error>> {
    // 1. Check environment variable
    if let Ok(addr) = std::env::var("IBUS_ADDRESS") {
        if !addr.is_empty() {
            return Ok(addr);
        }
    }

    // 2. Read from IBus socket file
    let machine_id = fs::read_to_string("/var/lib/dbus/machine-id")
        .or_else(|_| fs::read_to_string("/etc/machine-id"))
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

    let path = socket_file.ok_or("IBus socket file not found")?;
    let content = fs::read_to_string(&path)?;
    for line in content.lines() {
        let line = line.trim();
        if let Some(addr) = line.strip_prefix("IBUS_ADDRESS=") {
            return Ok(addr.to_string());
        }
    }

    Err("IBUS_ADDRESS not found in socket file".into())
}

/// Extract engine name from the IBusEngineDesc variant returned by GetGlobalEngine.
///
/// IBusEngineDesc is serialized as (sa{sv}sss...) where:
///   field 0: type name ("IBusEngineDesc")
///   field 1: properties dict
///   field 2: engine name  <-- this is what we want
fn extract_engine_name_from_value(value: &zvariant::Value<'_>) -> Option<String> {
    // The return of GetGlobalEngine is a variant wrapping a struct
    let st = match value {
        zvariant::Value::Structure(s) => s,
        _ => return None,
    };
    let fields = st.fields();
    // field[2] is the engine name
    if let Some(zvariant::Value::Str(name)) = fields.get(2) {
        return Some(name.to_string());
    }
    None
}

fn get_global_engine_name(proxy: &zbus::blocking::Proxy<'_>) -> Option<String> {
    let reply = proxy.call_method("GetGlobalEngine", &()).ok()?;
    let body = reply.body();
    let value: zvariant::Value = body.deserialize().ok()?;
    extract_engine_name_from_value(&value)
}

fn spawn_ibus_watcher(sender: glib::Sender<String>) {
    std::thread::spawn(move || {
        let addr_str = match discover_ibus_address() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Failed to discover IBus address: {}", e);
                return;
            }
        };

        // Build connection to the IBus-specific D-Bus address
        let addr: zbus::Address = match addr_str.as_str().try_into() {
            Ok(a) => a,
            Err(e) => {
                eprintln!("Failed to parse IBus address: {}", e);
                return;
            }
        };

        let async_conn = match async_io::block_on(
            zbus::connection::Builder::address(addr)
                .expect("Failed to create connection builder")
                .build(),
        ) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("Failed to connect to IBus: {}", e);
                return;
            }
        };
        let conn = zbus::blocking::Connection::from(async_conn);

        let proxy = match zbus::blocking::Proxy::new(
            &conn,
            "org.freedesktop.IBus",
            "/org/freedesktop/IBus",
            "org.freedesktop.IBus",
        ) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("Failed to create IBus proxy: {}", e);
                return;
            }
        };

        // Get initial engine
        if let Some(name) = get_global_engine_name(&proxy) {
            let label = label_from_engine(&name);
            let _ = sender.send(label.to_string());
        }

        // Poll for engine changes
        // Using polling because zbus blocking signal iteration has API limitations.
        let mut last_label = String::new();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(200));
            if let Some(name) = get_global_engine_name(&proxy) {
                let label = label_from_engine(&name).to_string();
                if label != last_label {
                    last_label.clone_from(&label);
                    let _ = sender.send(label);
                }
            }
        }
    });
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

    // Enable per-pixel alpha via RGBA visual
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    // Draw handler
    let state_draw = Rc::clone(&state);
    window.connect_draw(move |_widget, ctx| {
        let st = state_draw.borrow();
        let w = st.width as f64;
        let h = st.height as f64;

        // Clear to fully transparent
        ctx.set_operator(cairo::Operator::Source);
        ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        ctx.paint().expect("paint failed");
        ctx.set_operator(cairo::Operator::Over);

        // Draw rounded rectangle background
        let r = w.min(h) * 0.32;
        rounded_rect(ctx, 0.0, 0.0, w, h, r);

        if st.label == "\u{3042}" {
            // Japanese mode: red background
            ctx.set_source_rgba(0.8, 0.0, 0.0, st.opacity);
        } else {
            // English mode: black background
            ctx.set_source_rgba(0.0, 0.0, 0.0, st.opacity);
        }
        ctx.fill().expect("fill failed");

        // Draw centered text using Pango
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

    // Position at center of screen
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

    // Set up glib channel for receiving label updates from IBus watcher
    #[allow(deprecated)]
    let (sender, receiver) = glib::MainContext::channel(glib::Priority::DEFAULT);

    let state_recv = Rc::clone(&state);
    let window_recv = window.clone();
    receiver.attach(None, move |new_label: String| {
        {
            let mut st = state_recv.borrow_mut();
            st.label = new_label.clone();
        }
        if new_label == "A" {
            window_recv.hide();
        } else {
            window_recv.show_all();
            window_recv.queue_draw();
        }
        glib::ControlFlow::Continue
    });

    // Spawn IBus watcher thread
    spawn_ibus_watcher(sender);

    // Handle Ctrl+C
    glib::unix_signal_add_local(libc::SIGINT, || {
        gtk::main_quit();
        glib::ControlFlow::Break
    });

    gtk::main();
}
