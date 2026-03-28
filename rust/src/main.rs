mod config;
mod ibus;
mod label;
mod overlay;
mod tray;

use std::cell::RefCell;
use std::rc::Rc;

use clap::Parser;
use gdk::prelude::*;
use gtk::prelude::*;

use crate::config::{resolve_app_config, CliArgs};
use crate::ibus::setup_ibus;
use crate::overlay::{rounded_rect, OverlayState};
use crate::tray::TrayIndicator;

fn main() {
    let cli = CliArgs::parse();
    let app_cfg = resolve_app_config(cli);

    gtk::init().expect("Failed to initialize GTK");

    let off = app_cfg.off_style;
    let on = app_cfg.on_style;
    let state = Rc::new(RefCell::new(OverlayState {
        label: "A".to_string(),
        width: off.width,
        height: off.height,
        opacity: off.opacity,
        on_style: on,
        off_style: off,
        caret_x: 0,
        caret_y: 0,
        caret_h: 0,
        caret_known: false,
        offset_x: off.offset_x,
        offset_y: off.offset_y,
        poll_ms: app_cfg.poll_ms,
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

    let tray = TrayIndicator::new().map(Rc::new);

    let ibus_runtime = setup_ibus(&state, &window, tray.clone());

    glib::unix_signal_add_local(libc::SIGINT, || {
        gtk::main_quit();
        glib::ControlFlow::Break
    });
    glib::unix_signal_add_local(libc::SIGTERM, || {
        gtk::main_quit();
        glib::ControlFlow::Break
    });

    gtk::main();

    drop(ibus_runtime);
    drop(tray);
}
