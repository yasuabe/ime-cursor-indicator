use std::fs;
use std::path::PathBuf;

use clap::Parser;
use serde::Deserialize;

const DEFAULT_POLL_MS: u64 = 75;
const DEFAULT_OFFSET_X: i32 = 20;
const DEFAULT_OFFSET_Y: i32 = 18;
const DEFAULT_WIDTH: i32 = 34;
const DEFAULT_HEIGHT: i32 = 34;
const DEFAULT_OPACITY: f64 = 0.70;

#[derive(Clone, Copy)]
pub(crate) struct OverlayStyle {
    pub(crate) offset_x: i32,
    pub(crate) offset_y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) opacity: f64,
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
pub(crate) struct CliArgs {
    #[arg(long = "poll-ms")]
    pub(crate) poll_ms: Option<u64>,
    #[arg(long = "offset-x")]
    pub(crate) offset_x: Option<i32>,
    #[arg(long = "offset-y")]
    pub(crate) offset_y: Option<i32>,
    #[arg(long = "width")]
    pub(crate) width: Option<i32>,
    #[arg(long = "height")]
    pub(crate) height: Option<i32>,
    #[arg(long = "opacity")]
    pub(crate) opacity: Option<f64>,
}

pub(crate) struct AppConfig {
    pub(crate) poll_ms: u64,
    pub(crate) on_style: OverlayStyle,
    pub(crate) off_style: OverlayStyle,
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

pub(crate) fn resolve_app_config(cli: CliArgs) -> AppConfig {
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
