use std::cell::RefCell;
use std::f64::consts::PI;
use std::rc::Rc;
use std::time::Duration;

use gdk::prelude::*;
use gtk::prelude::*;

use crate::config::OverlayStyle;
use crate::tray::TrayIndicator;

pub(crate) struct OverlayState {
    pub(crate) label: String,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) opacity: f64,
    pub(crate) on_style: OverlayStyle,
    pub(crate) off_style: OverlayStyle,
    pub(crate) caret_x: i32,
    pub(crate) caret_y: i32,
    pub(crate) caret_h: i32,
    pub(crate) caret_known: bool,
    pub(crate) offset_x: i32,
    pub(crate) offset_y: i32,
    pub(crate) poll_ms: u64,
    pub(crate) pointer_poll_source: Option<glib::SourceId>,
    pub(crate) last_window_x: i32,
    pub(crate) last_window_y: i32,
}

pub(crate) fn rounded_rect(ctx: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    ctx.new_sub_path();
    ctx.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    ctx.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    ctx.arc(x + r, y + h - r, r, PI / 2.0, PI);
    ctx.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    ctx.close_path();
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
        clamp_to_monitor(
            &display, anchor_x, anchor_y, target_x, target_y, st.width, st.height,
        )
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
            st.pointer_poll_source.is_none() && !st.caret_known,
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
            !st.caret_known
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

fn style_for_label(state: &OverlayState, label: &str) -> OverlayStyle {
    if label == "\u{3042}" {
        state.on_style
    } else {
        state.off_style
    }
}

fn apply_style_for_label(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window, label: &str) {
    let style = {
        let st = state.borrow();
        style_for_label(&st, label)
    };

    let mut st = state.borrow_mut();
    let size_changed = st.width != style.width || st.height != style.height;
    st.width = style.width;
    st.height = style.height;
    st.opacity = style.opacity;
    st.offset_x = style.offset_x;
    st.offset_y = style.offset_y;
    if size_changed {
        st.last_window_x = i32::MIN;
        st.last_window_y = i32::MIN;
    }
    drop(st);

    if size_changed {
        window.resize(style.width, style.height);
    }
}

pub(crate) fn update_label(
    state: &Rc<RefCell<OverlayState>>,
    window: &gtk::Window,
    tray: Option<&Rc<TrayIndicator>>,
    label: &str,
) {
    {
        let mut st = state.borrow_mut();
        if st.label == label {
            return;
        }
        st.label = label.to_string();
    }
    if let Some(tray) = tray {
        tray.set_label(label);
    }
    apply_style_for_label(state, window, label);
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

pub(crate) fn mark_caret_unknown(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window) {
    {
        let mut st = state.borrow_mut();
        st.caret_known = false;
    }
    move_to_pointer(state, window);
    start_pointer_poll(state, window);
    if !window.is_visible() {
        window.show_all();
    }
}

pub(crate) fn move_to_caret(
    state: &Rc<RefCell<OverlayState>>,
    window: &gtk::Window,
    x: i32,
    y: i32,
    h: i32,
) {
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
    if !window.is_visible() {
        window.show_all();
    }
}
