use std::f64::consts::PI;

use gdk::prelude::*;
use gtk::prelude::*;

fn rounded_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    ctx.arc(x + r, y + h - r, r, PI / 2.0, PI);
    ctx.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    ctx.close_path();
}

fn main() {
    gtk::init().expect("Failed to initialize GTK");

    let width = 34;
    let height = 34;
    let opacity = 0.7;

    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_app_paintable(true);
    window.set_decorated(false);
    window.set_keep_above(true);
    window.stick();
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_type_hint(gdk::WindowTypeHint::Notification);
    window.set_default_size(width, height);

    // Enable per-pixel alpha via RGBA visual
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    window.connect_draw(move |_widget, ctx| {
        let w = width as f64;
        let h = height as f64;

        // Clear to fully transparent
        ctx.set_operator(cairo::Operator::Source);
        ctx.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        ctx.paint().expect("paint failed");
        ctx.set_operator(cairo::Operator::Over);

        // Draw rounded rectangle background
        let r = w.min(h) * 0.32;
        rounded_rect(ctx, 0.0, 0.0, w, h, r);
        ctx.set_source_rgba(0.0, 0.0, 0.0, opacity);
        ctx.fill().expect("fill failed");

        // Draw centered text using Pango
        let layout = pangocairo::functions::create_layout(ctx);
        let font_desc = pango::FontDescription::from_string("Sans Bold 16");
        layout.set_font_description(Some(&font_desc));
        layout.set_text("A");
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
            let x = geom.x() + (geom.width() - width) / 2;
            let y = geom.y() + (geom.height() - height) / 2;
            window.move_(x, y);
        }
    }

    window.show_all();

    // Handle Ctrl+C
    glib::unix_signal_add_local(libc::SIGINT, || {
        gtk::main_quit();
        glib::ControlFlow::Break
    });

    gtk::main();
}
