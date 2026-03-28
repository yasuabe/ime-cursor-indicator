use std::f64::consts::PI;
use std::ffi::{c_char, c_int, c_void, CString};
use std::fs;
use std::path::{Path, PathBuf};

use gtk::prelude::*;
use libloading::Library;

const TRAY_ICON_SIZE: i32 = 22;
const APP_INDICATOR_CATEGORY_APPLICATION_STATUS: c_int = 0;
const APP_INDICATOR_STATUS_ACTIVE: c_int = 1;

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

pub(crate) struct TrayIndicator {
    api: AppIndicatorApi,
    indicator: *mut c_void,
    icon_dir: PathBuf,
    icon_a_name: String,
    icon_ja_name: String,
    _menu: gtk::Menu,
    _quit_item: gtk::MenuItem,
}

impl TrayIndicator {
    pub(crate) fn new() -> Option<Self> {
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

    pub(crate) fn set_label(&self, label: &str) {
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
