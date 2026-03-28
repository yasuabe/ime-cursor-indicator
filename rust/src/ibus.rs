use std::cell::RefCell;
use std::fs;
use std::rc::Rc;

use futures_channel::mpsc;
use futures_util::stream::StreamExt;
use glib::ToVariant;

use crate::label::{label_from_engine, label_from_symbol};
use crate::overlay::{mark_caret_unknown, move_to_caret, update_label, OverlayState};
use crate::tray::TrayIndicator;

pub(crate) struct IBusRuntime {
    conn: gio::DBusConnection,
    signal_ids: Vec<gio::SignalSubscriptionId>,
    filter_id: Option<gio::FilterId>,
    caret_task: Option<glib::JoinHandle<()>>,
}

impl Drop for IBusRuntime {
    fn drop(&mut self) {
        if let Some(task) = self.caret_task.take() {
            task.abort();
        }
        for id in self.signal_ids.drain(..) {
            self.conn.signal_unsubscribe(id);
        }
        if let Some(id) = self.filter_id.take() {
            self.conn.remove_filter(id);
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

fn extract_engine_name(reply: &glib::Variant) -> Option<String> {
    let variant = reply.child_value(0);
    let engine_desc = variant.child_value(0);
    let name = engine_desc.try_child_value(2)?;
    Some(name.str()?.to_string())
}

fn extract_property_key_and_symbol(params: &glib::Variant) -> Option<(String, String)> {
    let prop_variant = params.child_value(0);
    let prop = prop_variant.child_value(0);
    let n = prop.n_children();
    if n < 12 {
        return None;
    }
    let key = prop.try_child_value(2)?.str()?.to_string();
    let symbol_variant = prop.try_child_value(11)?;
    let symbol_text = symbol_variant.child_value(0);
    if symbol_text.n_children() < 3 {
        return None;
    }
    let symbol = symbol_text.try_child_value(2)?.str()?.to_string();
    Some((key, symbol))
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

pub(crate) fn setup_ibus(
    state: &Rc<RefCell<OverlayState>>,
    window: &gtk::Window,
    tray: Option<Rc<TrayIndicator>>,
) -> Option<IBusRuntime> {
    let addr = match discover_ibus_address() {
        Some(a) => a,
        None => {
            eprintln!("Failed to discover IBus address");
            return None;
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
            return None;
        }
    };

    let mut signal_ids = Vec::new();

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
            update_label(state, window, tray.as_ref(), label);
        }
    }

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

    let state_eng = Rc::clone(state);
    let window_eng = window.clone();
    let tray_eng = tray.clone();
    let global_engine_changed_id = conn.signal_subscribe(
        None::<&str>,
        Some("org.freedesktop.IBus"),
        Some("GlobalEngineChanged"),
        Some("/org/freedesktop/IBus"),
        None::<&str>,
        gio::DBusSignalFlags::NONE,
        move |_conn, _sender, _path, _iface, _signal, params| {
            if let Some(name) = params.child_value(0).str() {
                let label = label_from_engine(&name);
                update_label(&state_eng, &window_eng, tray_eng.as_ref(), label);
            }
        },
    );
    signal_ids.push(global_engine_changed_id);

    let state_prop = Rc::clone(state);
    let window_prop = window.clone();
    let tray_prop = tray.clone();
    let update_property_id = conn.signal_subscribe(
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
                    update_label(&state_prop, &window_prop, tray_prop.as_ref(), label);
                }
            }
        },
    );
    signal_ids.push(update_property_id);

    let (caret_sender, mut caret_receiver) = mpsc::unbounded::<(i32, i32, i32, i32)>();
    let state_caret_rx = Rc::clone(state);
    let window_caret_rx = window.clone();
    let caret_task = glib::spawn_future_local(async move {
        while let Some((x, y, w, h)) = caret_receiver.next().await {
            if x == 0 && y == 0 && w == 0 && h == 0 {
                mark_caret_unknown(&state_caret_rx, &window_caret_rx);
            } else {
                move_to_caret(&state_caret_rx, &window_caret_rx, x, y, h);
            }
        }
    });

    let focused_ic_path = Rc::new(RefCell::new(None::<String>));
    let last_cursor = Rc::new(RefCell::new((-1, -1, -1, -1)));

    let caret_sender_filter = caret_sender.clone();
    let focused_ic_path_filter = Rc::clone(&focused_ic_path);
    let last_cursor_filter = Rc::clone(&last_cursor);
    let caret_filter_id = conn.add_filter(move |_conn, message, incoming| {
        if !incoming {
            return Some(message.clone());
        }
        if message.interface().as_deref() != Some("org.freedesktop.IBus.InputContext") {
            return Some(message.clone());
        }
        let msg_type = message.message_type();
        let is_method_or_signal = msg_type == gio::DBusMessageType::MethodCall
            || msg_type == gio::DBusMessageType::Signal;
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
                let _ = caret_sender_filter.unbounded_send((0, 0, 0, 0));
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
            let _ = caret_sender_filter.unbounded_send((0, 0, 0, 0));
            return Some(message.clone());
        }

        let coords = (x, y, w, h);
        if *last_cursor_filter.borrow() == coords {
            return Some(message.clone());
        }
        *last_cursor_filter.borrow_mut() = coords;

        let _ = caret_sender_filter.unbounded_send((x, y, w, h));

        Some(message.clone())
    });

    Some(IBusRuntime {
        conn,
        signal_ids,
        filter_id: Some(caret_filter_id),
        caret_task: Some(caret_task),
    })
}
