use std::cell::RefCell;
use std::f64::consts::PI;
use std::ffi::{c_char, c_int, c_void, CString};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use clap::Parser;
use futures_channel::mpsc;
use futures_util::stream::StreamExt;
use gdk::prelude::*;
use gtk::prelude::*;
use libloading::Library;
use serde::Deserialize;

const DEFAULT_POLL_MS: u64 = 75;
const DEFAULT_OFFSET_X: i32 = 20;
const DEFAULT_OFFSET_Y: i32 = 18;
const DEFAULT_WIDTH: i32 = 34;
const DEFAULT_HEIGHT: i32 = 34;
const DEFAULT_OPACITY: f64 = 0.70;
const TRAY_ICON_SIZE: i32 = 22;
const APP_INDICATOR_CATEGORY_APPLICATION_STATUS: c_int = 0;
const APP_INDICATOR_STATUS_ACTIVE: c_int = 1;

#[derive(Clone, Copy)]
struct OverlayStyle {
    offset_x: i32,
    offset_y: i32,
    width: i32,
    height: i32,
    opacity: f64,
}

#[derive(Clone, Copy, Default)]
struct StyleOverride {
    offset_x: Option<i32>,
    offset_y: Option<i32>,
    width: Option<i32>,
    height: Option<i32>,
    opacity: Option<f64>,
}

impl StyleOverride {
    fn apply_to(self, mut style: OverlayStyle) -> OverlayStyle {
        if let Some(v) = self.offset_x {
            style.offset_x = v;
        }
        if let Some(v) = self.offset_y {
            style.offset_y = v;
        }
        if let Some(v) = self.width {
            style.width = v;
        }
        if let Some(v) = self.height {
            style.height = v;
        }
        if let Some(v) = self.opacity {
            style.opacity = v;
        }
        style
    }
}

#[derive(Default, Deserialize)]
struct RawConfig {
    poll_ms: Option<toml::Value>,
    offset_x: Option<toml::Value>,
    offset_y: Option<toml::Value>,
    width: Option<toml::Value>,
    height: Option<toml::Value>,
    opacity: Option<toml::Value>,
    on: Option<toml::Value>,
    off: Option<toml::Value>,
}

#[derive(Parser, Debug)]
#[command(about = "Show IME status indicator near cursor on Ubuntu/X11 + IBus")]
struct CliArgs {
    #[arg(long = "poll-ms")]
    poll_ms: Option<u64>,
    #[arg(long = "offset-x")]
    offset_x: Option<i32>,
    #[arg(long = "offset-y")]
    offset_y: Option<i32>,
    #[arg(long = "width")]
    width: Option<i32>,
    #[arg(long = "height")]
    height: Option<i32>,
    #[arg(long = "opacity")]
    opacity: Option<f64>,
}

struct AppConfig {
    poll_ms: u64,
    on_style: OverlayStyle,
    off_style: OverlayStyle,
}

struct OverlayState {
    label: String,
    width: i32,
    height: i32,
    opacity: f64,
    on_style: OverlayStyle,
    off_style: OverlayStyle,
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

struct IBusRuntime {
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

type AppIndicatorNewFn = unsafe extern "C" fn(*const c_char, *const c_char, c_int) -> *mut c_void;
type AppIndicatorSetStatusFn = unsafe extern "C" fn(*mut c_void, c_int);
type AppIndicatorSetMenuFn = unsafe extern "C" fn(*mut c_void, *mut gtk::ffi::GtkMenu);
type AppIndicatorSetIconThemePathFn = unsafe extern "C" fn(*mut c_void, *const c_char);
type AppIndicatorSetIconFullFn = unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char);

struct AppIndicatorApi {
    _lib: Library,
    new_fn: AppIndicatorNewFn,
    set_status_fn: AppIndicatorSetStatusFn,
    set_menu_fn: AppIndicatorSetMenuFn,
    set_icon_theme_path_fn: AppIndicatorSetIconThemePathFn,
    set_icon_full_fn: AppIndicatorSetIconFullFn,
}

impl AppIndicatorApi {
    fn load() -> Result<Self, String> {
        let candidates = [
            "libayatana-appindicator3.so.1",
            "libayatana-appindicator3.so",
            "libappindicator3.so.1",
            "libappindicator3.so",
        ];
        for candidate in candidates {
            let lib = match unsafe { Library::new(candidate) } {
                Ok(lib) => lib,
                Err(_) => continue,
            };
            let loaded = unsafe {
                let new_fn = match lib.get::<AppIndicatorNewFn>(b"app_indicator_new\0") {
                    Ok(sym) => *sym,
                    Err(_) => continue,
                };
                let set_status_fn =
                    match lib.get::<AppIndicatorSetStatusFn>(b"app_indicator_set_status\0") {
                        Ok(sym) => *sym,
                        Err(_) => continue,
                    };
                let set_menu_fn =
                    match lib.get::<AppIndicatorSetMenuFn>(b"app_indicator_set_menu\0") {
                        Ok(sym) => *sym,
                        Err(_) => continue,
                    };
                let set_icon_theme_path_fn = match lib
                    .get::<AppIndicatorSetIconThemePathFn>(b"app_indicator_set_icon_theme_path\0")
                {
                    Ok(sym) => *sym,
                    Err(_) => continue,
                };
                let set_icon_full_fn =
                    match lib.get::<AppIndicatorSetIconFullFn>(b"app_indicator_set_icon_full\0") {
                        Ok(sym) => *sym,
                        Err(_) => continue,
                    };
                AppIndicatorApi {
                    _lib: lib,
                    new_fn,
                    set_status_fn,
                    set_menu_fn,
                    set_icon_theme_path_fn,
                    set_icon_full_fn,
                }
            };
            return Ok(loaded);
        }
        Err("Ayatana/AppIndicator shared library not found".to_string())
    }
}

struct TrayIndicator {
    api: AppIndicatorApi,
    indicator: *mut c_void,
    icon_dir: PathBuf,
    icon_a_name: String,
    icon_ja_name: String,
    _menu: gtk::Menu,
    _quit_item: gtk::MenuItem,
}

impl TrayIndicator {
    fn new() -> Option<Self> {
        let api = match AppIndicatorApi::load() {
            Ok(api) => api,
            Err(err) => {
                eprintln!("Warning: tray is disabled: {}", err);
                return None;
            }
        };

        let icon_dir = match create_tray_icon_dir() {
            Ok(dir) => dir,
            Err(err) => {
                eprintln!("Warning: tray icon init failed: {}", err);
                return None;
            }
        };
        let icon_a_name = "icon_a".to_string();
        let icon_ja_name = "icon_ja".to_string();
        if let Err(err) = create_tray_icon(
            &icon_dir.join(format!("{}.png", icon_a_name)),
            (0.0, 0.0, 0.0),
            "A",
        ) {
            eprintln!("Warning: tray icon init failed: {}", err);
            let _ = fs::remove_dir_all(&icon_dir);
            return None;
        }
        if let Err(err) = create_tray_icon(
            &icon_dir.join(format!("{}.png", icon_ja_name)),
            (0.8, 0.0, 0.0),
            "\u{3042}",
        ) {
            eprintln!("Warning: tray icon init failed: {}", err);
            let _ = fs::remove_dir_all(&icon_dir);
            return None;
        }

        let id = CString::new("ime-cursor-indicator").ok()?;
        let initial_icon = CString::new(icon_a_name.as_str()).ok()?;
        let icon_dir_c = CString::new(icon_dir.to_string_lossy().as_bytes()).ok()?;
        let indicator = unsafe {
            (api.new_fn)(
                id.as_ptr(),
                initial_icon.as_ptr(),
                APP_INDICATOR_CATEGORY_APPLICATION_STATUS,
            )
        };
        if indicator.is_null() {
            eprintln!("Warning: tray is disabled: app_indicator_new returned null");
            let _ = fs::remove_dir_all(&icon_dir);
            return None;
        }

        unsafe {
            (api.set_icon_theme_path_fn)(indicator, icon_dir_c.as_ptr());
            (api.set_status_fn)(indicator, APP_INDICATOR_STATUS_ACTIVE);
        }

        let menu = gtk::Menu::new();
        let quit_item = gtk::MenuItem::with_label("Quit");
        quit_item.connect_activate(|_| {
            gtk::main_quit();
        });
        menu.append(&quit_item);
        menu.show_all();
        unsafe {
            (api.set_menu_fn)(indicator, menu.as_ptr() as *mut gtk::ffi::GtkMenu);
        }

        let tray = Self {
            api,
            indicator,
            icon_dir,
            icon_a_name,
            icon_ja_name,
            _menu: menu,
            _quit_item: quit_item,
        };
        tray.set_label("A");
        Some(tray)
    }

    fn set_label(&self, label: &str) {
        let icon_name = if label == "\u{3042}" {
            self.icon_ja_name.as_str()
        } else {
            self.icon_a_name.as_str()
        };
        let Ok(icon_c) = CString::new(icon_name) else {
            return;
        };
        let Ok(desc_c) = CString::new(label) else {
            return;
        };
        unsafe {
            (self.api.set_icon_full_fn)(self.indicator, icon_c.as_ptr(), desc_c.as_ptr());
        }
    }
}

impl Drop for TrayIndicator {
    fn drop(&mut self) {
        if !self.indicator.is_null() {
            unsafe {
                glib::gobject_ffi::g_object_unref(self.indicator as *mut glib::gobject_ffi::GObject)
            };
        }
        if let Err(err) = fs::remove_dir_all(&self.icon_dir) {
            eprintln!(
                "Warning: failed to clean tray icon dir {}: {}",
                self.icon_dir.display(),
                err
            );
        }
    }
}

fn create_tray_icon_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!(
        "ime-indicator-{}-{}",
        std::process::id(),
        glib::monotonic_time()
    ));
    fs::create_dir_all(&dir)
        .map_err(|err| format!("failed to create {}: {}", dir.display(), err))?;
    Ok(dir)
}

fn create_tray_icon(path: &Path, rgb: (f64, f64, f64), label: &str) -> Result<(), String> {
    let surface =
        cairo::ImageSurface::create(cairo::Format::ARgb32, TRAY_ICON_SIZE, TRAY_ICON_SIZE)
            .map_err(|err| format!("surface create failed: {}", err))?;
    let ctx =
        cairo::Context::new(&surface).map_err(|err| format!("context create failed: {}", err))?;

    let size = TRAY_ICON_SIZE as f64;
    ctx.set_source_rgb(rgb.0, rgb.1, rgb.2);
    ctx.arc(size / 2.0, size / 2.0, size / 2.0, 0.0, PI * 2.0);
    ctx.fill().map_err(|err| format!("fill failed: {}", err))?;

    let layout = pangocairo::functions::create_layout(&ctx);
    let font_desc = pango::FontDescription::from_string("Sans Bold 14");
    layout.set_font_description(Some(&font_desc));
    layout.set_text(label);
    let (_, logical) = layout.pixel_extents();
    let tx = (TRAY_ICON_SIZE - logical.width()) / 2 - logical.x();
    let ty = (TRAY_ICON_SIZE - logical.height()) / 2 - logical.y();
    ctx.move_to(tx as f64, ty as f64);
    ctx.set_source_rgb(1.0, 1.0, 1.0);
    pangocairo::functions::show_layout(&ctx, &layout);

    let mut file = std::fs::File::create(path)
        .map_err(|err| format!("png create failed ({}): {}", path.display(), err))?;
    surface
        .write_to_png(&mut file)
        .map_err(|err| format!("png write failed ({}): {}", path.display(), err))
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

fn default_style() -> OverlayStyle {
    OverlayStyle {
        offset_x: DEFAULT_OFFSET_X,
        offset_y: DEFAULT_OFFSET_Y,
        width: DEFAULT_WIDTH,
        height: DEFAULT_HEIGHT,
        opacity: DEFAULT_OPACITY,
    }
}

fn config_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    PathBuf::from(home)
        .join(".config")
        .join("ime-cursor-indicator")
        .join("config.toml")
}

fn parse_i32_value(value: &toml::Value, label: &str) -> Option<i32> {
    match value {
        toml::Value::Integer(v) => i32::try_from(*v).ok().or_else(|| {
            eprintln!(
                "Warning: config {} is out of i32 range ({}); ignoring",
                label, v
            );
            None
        }),
        _ => {
            eprintln!(
                "Warning: config {} should be integer, got {}; ignoring",
                label,
                value.type_str()
            );
            None
        }
    }
}

fn parse_positive_i32_value(value: &toml::Value, label: &str) -> Option<i32> {
    let v = parse_i32_value(value, label)?;
    if v <= 0 {
        eprintln!(
            "Warning: config {} should be > 0, got {}; ignoring",
            label, v
        );
        None
    } else {
        Some(v)
    }
}

fn parse_poll_ms_value(value: &toml::Value, label: &str) -> Option<u64> {
    match value {
        toml::Value::Integer(v) => {
            if *v <= 0 {
                eprintln!(
                    "Warning: config {} should be > 0, got {}; ignoring",
                    label, v
                );
                return None;
            }
            u64::try_from(*v).ok().or_else(|| {
                eprintln!(
                    "Warning: config {} is out of u64 range ({}); ignoring",
                    label, v
                );
                None
            })
        }
        _ => {
            eprintln!(
                "Warning: config {} should be integer, got {}; ignoring",
                label,
                value.type_str()
            );
            None
        }
    }
}

fn parse_opacity_value(value: &toml::Value, label: &str) -> Option<f64> {
    let raw = match value {
        toml::Value::Float(v) => *v,
        toml::Value::Integer(v) => *v as f64,
        _ => {
            eprintln!(
                "Warning: config {} should be float, got {}; ignoring",
                label,
                value.type_str()
            );
            return None;
        }
    };
    Some(raw.clamp(0.1, 1.0))
}

fn parse_style_override_table(table: &toml::Table, section: Option<&str>) -> StyleOverride {
    let mut style = StyleOverride::default();
    let key_label = |key: &str| match section {
        Some(sec) => format!("[{}].{}", sec, key),
        None => key.to_string(),
    };

    if let Some(v) = table
        .get("offset_x")
        .and_then(|v| parse_i32_value(v, &key_label("offset_x")))
    {
        style.offset_x = Some(v);
    }
    if let Some(v) = table
        .get("offset_y")
        .and_then(|v| parse_i32_value(v, &key_label("offset_y")))
    {
        style.offset_y = Some(v);
    }
    if let Some(v) = table
        .get("width")
        .and_then(|v| parse_positive_i32_value(v, &key_label("width")))
    {
        style.width = Some(v);
    }
    if let Some(v) = table
        .get("height")
        .and_then(|v| parse_positive_i32_value(v, &key_label("height")))
    {
        style.height = Some(v);
    }
    if let Some(v) = table
        .get("opacity")
        .and_then(|v| parse_opacity_value(v, &key_label("opacity")))
    {
        style.opacity = Some(v);
    }

    style
}

fn load_file_config() -> (Option<u64>, StyleOverride, StyleOverride, StyleOverride) {
    let path = config_file_path();
    if !path.exists() {
        return (
            None,
            StyleOverride::default(),
            StyleOverride::default(),
            StyleOverride::default(),
        );
    }

    let content = match fs::read_to_string(&path) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("Warning: failed to load {}: {}", path.display(), err);
            return (
                None,
                StyleOverride::default(),
                StyleOverride::default(),
                StyleOverride::default(),
            );
        }
    };

    let raw: RawConfig = match toml::from_str(&content) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("Warning: failed to parse {}: {}", path.display(), err);
            return (
                None,
                StyleOverride::default(),
                StyleOverride::default(),
                StyleOverride::default(),
            );
        }
    };

    let mut top = StyleOverride::default();
    let poll_ms = raw
        .poll_ms
        .as_ref()
        .and_then(|v| parse_poll_ms_value(v, "poll_ms"));
    if let Some(v) = raw
        .offset_x
        .as_ref()
        .and_then(|v| parse_i32_value(v, "offset_x"))
    {
        top.offset_x = Some(v);
    }
    if let Some(v) = raw
        .offset_y
        .as_ref()
        .and_then(|v| parse_i32_value(v, "offset_y"))
    {
        top.offset_y = Some(v);
    }
    if let Some(v) = raw
        .width
        .as_ref()
        .and_then(|v| parse_positive_i32_value(v, "width"))
    {
        top.width = Some(v);
    }
    if let Some(v) = raw
        .height
        .as_ref()
        .and_then(|v| parse_positive_i32_value(v, "height"))
    {
        top.height = Some(v);
    }
    if let Some(v) = raw
        .opacity
        .as_ref()
        .and_then(|v| parse_opacity_value(v, "opacity"))
    {
        top.opacity = Some(v);
    }

    let mut on = StyleOverride::default();
    let mut off = StyleOverride::default();
    if let Some(value) = raw.on {
        match value {
            toml::Value::Table(table) => on = parse_style_override_table(&table, Some("on")),
            other => eprintln!(
                "Warning: config [on] should be a table, got {}; ignoring",
                other.type_str()
            ),
        }
    }
    if let Some(value) = raw.off {
        match value {
            toml::Value::Table(table) => off = parse_style_override_table(&table, Some("off")),
            other => eprintln!(
                "Warning: config [off] should be a table, got {}; ignoring",
                other.type_str()
            ),
        }
    }

    (poll_ms, top, on, off)
}

fn resolve_app_config(cli: CliArgs) -> AppConfig {
    let (file_poll_ms, file_top, file_on, file_off) = load_file_config();

    let mut poll_ms = file_poll_ms.unwrap_or(DEFAULT_POLL_MS);
    if let Some(v) = cli.poll_ms {
        poll_ms = v.max(1);
    }

    let mut base = default_style();
    base = file_top.apply_to(base);
    let cli_top = StyleOverride {
        offset_x: cli.offset_x,
        offset_y: cli.offset_y,
        width: cli.width.filter(|v| *v > 0),
        height: cli.height.filter(|v| *v > 0),
        opacity: cli.opacity.map(|v| v.clamp(0.1, 1.0)),
    };
    if cli.width.is_some() && cli_top.width.is_none() {
        eprintln!("Warning: CLI --width should be > 0; ignoring");
    }
    if cli.height.is_some() && cli_top.height.is_none() {
        eprintln!("Warning: CLI --height should be > 0; ignoring");
    }
    base = cli_top.apply_to(base);

    let on_style = file_on.apply_to(base);
    let off_style = file_off.apply_to(base);

    AppConfig {
        poll_ms,
        on_style,
        off_style,
    }
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

fn mark_caret_unknown(state: &Rc<RefCell<OverlayState>>, window: &gtk::Window) {
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
    if !window.is_visible() {
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

fn update_label(
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

fn setup_ibus(
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
            update_label(state, window, tray.as_ref(), label);
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

    // Subscribe to UpdateProperty (eavesdrop — match rule already added)
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

    // Track active context path to ignore noise from unfocused InputContexts.
    let focused_ic_path = Rc::new(RefCell::new(None::<String>));
    let last_cursor = Rc::new(RefCell::new((-1, -1, -1, -1)));

    // Intercept InputContext method calls via filter (eavesdrop rules already added)
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
