//! Native desktop ("computer use") engine.
//!
//! Engines (pinned, open source):
//!   - `xcap` 0.9.8 (MIT, https://github.com/nashaofu/xcap) — screen capture.
//!   - `enigo` 0.6.1 (MIT, https://github.com/enigo-rs/enigo) — mouse/keyboard.
//!   - `image` 0.25.10 (MIT) — PNG encode/downscale only.
//!
//! Coordinate space: physical pixels relative to the PRIMARY monitor's
//! top-left corner, so the space the model sees in `computer_screenshot` is
//! exactly the space `computer_click`/`computer_move` act in. `Screenshot`
//! carries the real monitor dimensions; the PNG itself may be downscaled to
//! the longest edge `MAX_EDGE_PX` (model vision prefers <=1600px anyway) —
//! the reported width/height stay the true monitor pixel dimensions for
//! coordinate mapping.
//!
//! Platform honesty:
//!   - Windows: full input (SendInput) + capture (GDI/WGC via xcap).
//!   - Linux X11: full input (XTEST via x11rb) + capture (xcb via xcap).
//!   - Linux Wayland: input REJECTED with an explicit reason — XTEST only
//!     reaches X11 (XWayland) clients, so pretending support would misdirect
//!     clicks into the wrong windows. Capture still tries xcap (wlr-screencopy
//!     / PipeWire portal); the first capture may raise the desktop's
//!     screen-share permission dialog, which is the platform's own consent
//!     surface.
//!   - macOS: code path exists; enigo requires Accessibility + Screen
//!     Recording permissions — failures surface that instruction.
//!   - No display server: clear error, no silent success.
//!
//! Safety bounds: arguments are schema-strict (unknown fields rejected),
//! validated before any OS call, coordinates must land inside the primary
//! monitor, text and scroll amounts are bounded, key combos are an allowlist
//! that is always released before the call returns, and no call sleeps.
//! Every call opens and drops its own handles — no global or non-Send state
//! is kept between calls; the caller serializes calls (spawn_blocking +
//! single-flight) so enigo/xcap never need to be Sync.

use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
use image::ImageEncoder as _;
use serde_json::{json, Map, Value};

/// Result of one computer-use call: either a small JSON description or a
/// screenshot whose `width`/`height` report the REAL primary-monitor pixel
/// dimensions (the PNG may be downscaled — see module docs).
pub enum ComputerOutput {
    Json(Value),
    Screenshot {
        png: Vec<u8>,
        width: u32,
        height: u32,
    },
}

const MAX_TEXT_CHARS: usize = 4096;
const MAX_PNG_BYTES: usize = 4 * 1024 * 1024;
const MAX_EDGE_PX: u32 = 1600;
const MAX_MONITOR_EDGE: u32 = 32768;
const MAX_SCROLL_STEPS: i64 = 240;
const MAX_COMBO_PARTS: usize = 5;
const MAX_MONITORS: usize = 32;

const WAYLAND_INPUT_ERROR: &str = "Desktop input is not supported on Wayland sessions: the XTEST protocol only reaches X11 (XWayland) applications, not Wayland-native windows, so clicks and keystrokes would silently land in the wrong place. Screenshots may still work. Run the app under an X11 session or on Windows for computer control.";
const HEADLESS_INPUT_ERROR: &str = "No graphical desktop session detected (DISPLAY is unset). Desktop input needs a running desktop session.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Session {
    Windows,
    Macos,
    X11,
    Wayland,
    Headless,
}

impl Session {
    fn detect() -> Self {
        if cfg!(windows) {
            return Session::Windows;
        }
        if cfg!(target_os = "macos") {
            return Session::Macos;
        }
        match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
            Some("wayland") => Session::Wayland,
            Some("x11") => Session::X11,
            _ => {
                if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                    Session::Wayland
                } else if std::env::var_os("DISPLAY").is_some() {
                    Session::X11
                } else {
                    Session::Headless
                }
            }
        }
    }
    fn name(self) -> &'static str {
        match self {
            Session::Windows => "windows",
            Session::Macos => "macos",
            Session::X11 => "x11",
            Session::Wayland => "wayland",
            Session::Headless => "headless",
        }
    }
}

/// Input gate. Anything that mutates the desktop goes through this first.
fn input_allowed(session: Session) -> Result<(), String> {
    match session {
        Session::Windows | Session::Macos | Session::X11 => Ok(()),
        Session::Wayland => Err(WAYLAND_INPUT_ERROR.into()),
        Session::Headless => Err(HEADLESS_INPUT_ERROR.into()),
    }
}

fn enigo_error(error: impl std::fmt::Display) -> String {
    let text = error.to_string();
    if cfg!(target_os = "macos") {
        format!("{text}. On macOS, grant this app Accessibility and Screen Recording permissions in System Settings → Privacy & Security, then retry.")
    } else {
        text
    }
}

#[derive(Clone, Copy, Debug)]
struct Mon {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f32,
    primary: bool,
}

/// Monitor enumeration via xcap (read-only; no capture, no input).
fn monitors() -> Result<Vec<Mon>, String> {
    let list =
        xcap::Monitor::all().map_err(|error| format!("Could not enumerate displays: {error}"))?;
    if list.is_empty() {
        return Err("No display monitors were reported by the operating system.".into());
    }
    if list.len() > MAX_MONITORS {
        return Err(format!("More than {MAX_MONITORS} monitors reported; refusing to operate on an unbounded desktop."));
    }
    let mut out = Vec::with_capacity(list.len());
    for monitor in &list {
        let x = monitor.x().map_err(|e| e.to_string())?;
        let y = monitor.y().map_err(|e| e.to_string())?;
        let width = monitor.width().map_err(|e| e.to_string())?;
        let height = monitor.height().map_err(|e| e.to_string())?;
        if width == 0 || height == 0 || width > MAX_MONITOR_EDGE || height > MAX_MONITOR_EDGE {
            return Err(format!(
                "Display reports an invalid size ({width}x{height})."
            ));
        }
        out.push(Mon {
            x,
            y,
            width,
            height,
            scale: monitor.scale_factor().unwrap_or(1.0),
            primary: monitor.is_primary().unwrap_or(false),
        });
    }
    Ok(out)
}

fn primary_mon() -> Result<Mon, String> {
    let list = monitors()?;
    Ok(list
        .iter()
        .copied()
        .find(|monitor| monitor.primary)
        .unwrap_or(list[0]))
}

/// The xcap monitor handle matching `mon` (needed for capture_image).
fn primary_xcap() -> Result<(xcap::Monitor, Mon), String> {
    let mon = primary_mon()?;
    let list =
        xcap::Monitor::all().map_err(|error| format!("Could not enumerate displays: {error}"))?;
    // Match on the geometry tuple we already validated — ids are not stable
    // across enumeration calls on every backend.
    for monitor in list {
        let (x, y, w, h) = (
            monitor.x().unwrap_or_default(),
            monitor.y().unwrap_or_default(),
            monitor.width().unwrap_or_default(),
            monitor.height().unwrap_or_default(),
        );
        if x == mon.x && y == mon.y && w == mon.width && h == mon.height {
            return Ok((monitor, mon));
        }
    }
    Err("The primary display disappeared between enumeration calls.".into())
}

/// Monitor bounds for coordinate validation. Prefers xcap; falls back to
/// enigo's own main-display size so input still works if xcap is broken.
fn bounds_with(enigo: &mut Enigo) -> Result<Mon, String> {
    match primary_mon() {
        Ok(mon) => Ok(mon),
        Err(_) => {
            let (width, height) = enigo.main_display().map_err(enigo_error)?;
            if width <= 0 || height <= 0 {
                return Err("The display reported an empty size.".into());
            }
            Ok(Mon {
                x: 0,
                y: 0,
                width: width as u32,
                height: height as u32,
                scale: 1.0,
                primary: true,
            })
        }
    }
}

fn check_point(x: i64, y: i64, mon: &Mon) -> Result<(), String> {
    if !(0..mon.width as i64).contains(&x) || !(0..mon.height as i64).contains(&y) {
        return Err(format!(
            "Coordinates ({x},{y}) are outside the primary monitor ({}x{}). Use values within those bounds.",
            mon.width, mon.height
        ));
    }
    Ok(())
}

fn png_encode(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| format!("Could not encode the screenshot as PNG: {error}"))?;
    Ok(out)
}

fn capture() -> Result<ComputerOutput, String> {
    let (monitor, _mon) = primary_xcap()?;
    let raw = monitor.capture_image().map_err(|error| {
        let hint = if Session::detect() == Session::Wayland {
            " Wayland capture depends on the compositor's screencopy/portal support; the desktop may have shown a screen-share prompt. Use an X11 session if capture keeps failing."
        } else {
            ""
        };
        format!("Screen capture failed: {error}.{hint}")
    })?;
    let (width, height) = (raw.width(), raw.height());
    if width == 0 || height == 0 || width > MAX_MONITOR_EDGE || height > MAX_MONITOR_EDGE {
        return Err(format!(
            "The display produced an invalid frame ({width}x{height})."
        ));
    }
    // Downscale the PNG to the model-friendly edge bound; width/height in the
    // output stay the true monitor pixels so coordinates keep mapping 1:1.
    let mut scaled = if width.max(height) > MAX_EDGE_PX {
        let (nw, nh) = scaled_fit(width, height, MAX_EDGE_PX);
        image::imageops::resize(&raw, nw, nh, image::imageops::FilterType::Triangle)
    } else {
        raw
    };
    let mut png = png_encode(&scaled)?;
    if png.len() > MAX_PNG_BYTES {
        let (nw, nh) = scaled_fit(scaled.width(), scaled.height(), MAX_EDGE_PX / 2);
        scaled = image::imageops::resize(
            &scaled,
            nw.max(1),
            nh.max(1),
            image::imageops::FilterType::Triangle,
        );
        png = png_encode(&scaled)?;
        if png.len() > MAX_PNG_BYTES {
            return Err(format!("Screenshot exceeds {MAX_PNG_BYTES} bytes even after downscaling; refusing to return an unbounded image."));
        }
    }
    Ok(ComputerOutput::Screenshot { png, width, height })
}

/// Downscale-to-fit preserving aspect; never upscales (callers only shrink).
fn scaled_fit(width: u32, height: u32, edge: u32) -> (u32, u32) {
    let longest = width.max(height).max(1) as u64;
    let nw = ((width as u64 * edge as u64 / longest).max(1) as u32).min(width);
    let nh = ((height as u64 * edge as u64 / longest).max(1) as u32).min(height);
    (nw, nh)
}

// ---------- argument validation (pure, runs before any OS handle) ----------

fn reject_unknown(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    for key in args.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(format!("Unknown argument '{key}' for this tool."));
        }
    }
    Ok(())
}

fn req_i64(args: &Map<String, Value>, key: &str, min: i64, max: i64) -> Result<i64, String> {
    let value = args
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| format!("'{key}' must be an integer."))?;
    if !(min..=max).contains(&value) {
        return Err(format!("'{key}' must be between {min} and {max}."));
    }
    Ok(value)
}

fn req_str(args: &Map<String, Value>, key: &str, min: usize, max: usize) -> Result<String, String> {
    let text = args
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("'{key}' must be a string."))?;
    if text.len() < min || text.len() > max {
        return Err(format!("'{key}' must be {min}-{max} bytes."));
    }
    Ok(text.to_string())
}

fn opt_bool(args: &Map<String, Value>, key: &str) -> Result<bool, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err(format!("'{key}' must be a boolean.")),
    }
}

// ---------- key combo parsing ----------

fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "enter" | "return" => Key::Return,
        "tab" => Key::Tab,
        "esc" | "escape" => Key::Escape,
        "space" => Key::Space,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "insert" | "ins" => Key::Insert,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" | "pgup" => Key::PageUp,
        "pagedown" | "pgdn" => Key::PageDown,
        "up" => Key::UpArrow,
        "down" => Key::DownArrow,
        "left" => Key::LeftArrow,
        "right" => Key::RightArrow,
        "capslock" => Key::CapsLock,
        "scrolllock" => Key::ScrollLock,
        "printscreen" | "printscr" | "prtsc" => Key::PrintScr,
        "f1" => Key::F1,
        "f2" => Key::F2,
        "f3" => Key::F3,
        "f4" => Key::F4,
        "f5" => Key::F5,
        "f6" => Key::F6,
        "f7" => Key::F7,
        "f8" => Key::F8,
        "f9" => Key::F9,
        "f10" => Key::F10,
        "f11" => Key::F11,
        "f12" => Key::F12,
        _ => return None,
    })
}

fn modifier_key(name: &str) -> Option<Key> {
    Some(match name {
        "ctrl" | "control" | "ctl" => Key::Control,
        "alt" | "option" => Key::Alt,
        "shift" => Key::Shift,
        "meta" | "super" | "win" | "cmd" | "command" => Key::Meta,
        _ => return None,
    })
}

/// Parse `"ctrl+c"` / `"enter"` / `"alt+f4"` into held modifiers + one key.
/// At most one non-modifier key; anything else is rejected, never guessed.
fn parse_key_combo(spec: &str) -> Result<(Vec<Key>, Key), String> {
    if spec.is_empty() || spec.len() > 64 || !spec.is_ascii() {
        return Err(
            "'keys' must be 1-64 ASCII characters such as \"enter\", \"ctrl+c\", or \"alt+f4\"."
                .into(),
        );
    }
    let raw_parts: Vec<&str> = spec.split('+').collect();
    if raw_parts.len() > MAX_COMBO_PARTS {
        return Err(format!("Key combos take at most {MAX_COMBO_PARTS} parts."));
    }
    let mut parts: Vec<String> = Vec::new();
    for part in raw_parts {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    // `ctrl++` presses Ctrl and the '+' key; a lone `+` is just that key.
    if spec.ends_with("++") || (parts.is_empty() && spec == "+") {
        parts.push("+".into());
    }
    if parts.is_empty() || parts.len() > MAX_COMBO_PARTS {
        return Err("Key combos take 1-5 parts, e.g. \"ctrl+shift+t\".".into());
    }
    let mut modifiers: Vec<Key> = Vec::new();
    let last = parts.len() - 1;
    for (index, part) in parts.iter().enumerate() {
        let lower = part.to_ascii_lowercase();
        if let Some(modi) = modifier_key(&lower) {
            if index == last {
                return Err(
                    "A key combo needs a non-modifier key at the end, e.g. \"ctrl+c\".".into(),
                );
            }
            if modifiers.contains(&modi) {
                return Err(format!("Duplicate modifier '{part}' in the key combo."));
            }
            modifiers.push(modi);
            continue;
        }
        if index != last {
            return Err(format!(
                "'{part}' is not a modifier; only ctrl/alt/shift/meta may lead a combo."
            ));
        }
        let key = if part == "+" {
            Key::Unicode('+')
        } else if let Some(named) = named_key(&lower) {
            named
        } else {
            let mut chars = part.chars();
            match (chars.next(), chars.next()) {
                // Lowercase single letters: typing 'C' makes the driver press
                // Shift itself, which would turn ctrl+c into ctrl+shift+c.
                (Some(c), None) if c.is_ascii_alphabetic() => Key::Unicode(c.to_ascii_lowercase()),
                (Some(c), None) => Key::Unicode(c),
                _ => return Err(format!("Unknown key name '{part}'. Use names like enter, tab, esc, space, backspace, delete, home, end, pageup, pagedown, arrows, f1-f12, or a single character.")),
            }
        };
        return Ok((modifiers, key));
    }
    Err("Invalid key combo.".into())
}

// ---------- actions ----------

fn with_enigo<R>(action: impl FnOnce(&mut Enigo, &Mon) -> Result<R, String>) -> Result<R, String> {
    input_allowed(Session::detect())?;
    let mut enigo = Enigo::new(&Settings::default()).map_err(enigo_error)?;
    let mon = bounds_with(&mut enigo)?;
    action(&mut enigo, &mon)
}

fn call_in(
    session: Session,
    name: &str,
    args: &Map<String, Value>,
) -> Result<ComputerOutput, String> {
    match name {
        "computer_screenshot" => {
            reject_unknown(args, &[])?;
            capture()
        }
        "computer_position" => {
            reject_unknown(args, &[])?;
            input_allowed(session)?;
            with_enigo(|enigo, mon| {
                let (gx, gy) = enigo.location().map_err(enigo_error)?;
                let inside = gx >= mon.x
                    && gy >= mon.y
                    && gx < mon.x + mon.width as i32
                    && gy < mon.y + mon.height as i32;
                Ok(ComputerOutput::Json(json!({
                    "x": gx - mon.x,
                    "y": gy - mon.y,
                    "insidePrimary": inside,
                    "monitor": {"x": mon.x, "y": mon.y, "width": mon.width, "height": mon.height},
                })))
            })
        }
        "computer_move" => {
            reject_unknown(args, &["x", "y"])?;
            let (x, y) = (
                req_i64(args, "x", 0, MAX_MONITOR_EDGE as i64)?,
                req_i64(args, "y", 0, MAX_MONITOR_EDGE as i64)?,
            );
            input_allowed(session)?;
            with_enigo(|enigo, mon| {
                check_point(x, y, mon)?;
                enigo
                    .move_mouse(mon.x + x as i32, mon.y + y as i32, Coordinate::Abs)
                    .map_err(enigo_error)?;
                Ok(ComputerOutput::Json(json!({"moved": true, "x": x, "y": y})))
            })
        }
        "computer_click" => {
            reject_unknown(args, &["x", "y", "button", "doubleClick"])?;
            let (x, y) = (
                req_i64(args, "x", 0, MAX_MONITOR_EDGE as i64)?,
                req_i64(args, "y", 0, MAX_MONITOR_EDGE as i64)?,
            );
            let button_name = match args.get("button") {
                None | Some(Value::Null) => "left".to_string(),
                Some(Value::String(name)) if !name.is_empty() && name.len() <= 8 => name.clone(),
                _ => return Err("'button' must be \"left\", \"right\", or \"middle\".".into()),
            };
            let button = match button_name.as_str() {
                "left" => Button::Left,
                "right" => Button::Right,
                "middle" => Button::Middle,
                other => {
                    return Err(format!(
                        "Unknown mouse button '{other}'. Use left, right, or middle."
                    ))
                }
            };
            let double = opt_bool(args, "doubleClick")?;
            input_allowed(session)?;
            with_enigo(|enigo, mon| {
                check_point(x, y, mon)?;
                enigo
                    .move_mouse(mon.x + x as i32, mon.y + y as i32, Coordinate::Abs)
                    .map_err(enigo_error)?;
                enigo
                    .button(button, Direction::Click)
                    .map_err(enigo_error)?;
                if double {
                    enigo
                        .button(button, Direction::Click)
                        .map_err(enigo_error)?;
                }
                Ok(ComputerOutput::Json(
                    json!({"clicked": true, "x": x, "y": y, "doubleClick": double}),
                ))
            })
        }
        "computer_type" => {
            reject_unknown(args, &["text"])?;
            let text = req_str(args, "text", 0, MAX_TEXT_CHARS)?;
            if text.contains('\0') {
                return Err("'text' cannot contain NUL bytes.".into());
            }
            input_allowed(session)?;
            with_enigo(|enigo, _mon| {
                enigo.text(&text).map_err(enigo_error)?;
                Ok(ComputerOutput::Json(
                    json!({"typed": true, "chars": text.chars().count()}),
                ))
            })
        }
        "computer_key" => {
            reject_unknown(args, &["keys"])?;
            let spec = req_str(args, "keys", 1, 64)?;
            let (modifiers, key) = parse_key_combo(&spec)?;
            input_allowed(session)?;
            with_enigo(|enigo, _mon| {
                let mut held: Vec<Key> = Vec::new();
                let mut failure: Option<String> = None;
                for modifier in &modifiers {
                    if let Err(error) = enigo.key(*modifier, Direction::Press) {
                        failure = Some(enigo_error(error));
                        break;
                    }
                    held.push(*modifier);
                }
                if failure.is_none() {
                    if let Err(error) = enigo.key(key, Direction::Click) {
                        failure = Some(enigo_error(error));
                    }
                }
                // Always release whatever was pressed, even on failure — no
                // key is ever held across calls.
                for modifier in held.iter().rev() {
                    let _ = enigo.key(*modifier, Direction::Release);
                }
                if let Some(error) = failure {
                    return Err(error);
                }
                Ok(ComputerOutput::Json(json!({"pressed": spec})))
            })
        }
        "computer_scroll" => {
            reject_unknown(args, &["dx", "dy", "x", "y"])?;
            let dy = req_i64(args, "dy", -MAX_SCROLL_STEPS, MAX_SCROLL_STEPS)?;
            let dx = match args.get("dx") {
                None | Some(Value::Null) => 0,
                Some(_) => req_i64(args, "dx", -MAX_SCROLL_STEPS, MAX_SCROLL_STEPS)?,
            };
            let pos = match (args.get("x"), args.get("y")) {
                (None, None) => None,
                (Some(_), Some(_)) => Some((
                    req_i64(args, "x", 0, MAX_MONITOR_EDGE as i64)?,
                    req_i64(args, "y", 0, MAX_MONITOR_EDGE as i64)?,
                )),
                _ => return Err("'x' and 'y' must be provided together or not at all.".into()),
            };
            input_allowed(session)?;
            with_enigo(|enigo, mon| {
                if let Some((x, y)) = pos {
                    check_point(x, y, mon)?;
                    enigo
                        .move_mouse(mon.x + x as i32, mon.y + y as i32, Coordinate::Abs)
                        .map_err(enigo_error)?;
                }
                if dy != 0 {
                    enigo
                        .scroll(dy as i32, Axis::Vertical)
                        .map_err(enigo_error)?;
                }
                if dx != 0 {
                    enigo
                        .scroll(dx as i32, Axis::Horizontal)
                        .map_err(enigo_error)?;
                }
                Ok(ComputerOutput::Json(
                    json!({"scrolled": true, "dx": dx, "dy": dy}),
                ))
            })
        }
        _ => Err(format!("Unknown computer tool '{name}'.")),
    }
}

/// Synchronous, blocking entry point for one computer-use call. The caller
/// runs this via `spawn_blocking` with a timeout; native calls are serialized
/// there.
pub fn call(name: &str, arguments: Value) -> Result<ComputerOutput, String> {
    let args = arguments
        .as_object()
        .ok_or("Tool arguments must be a JSON object.")?;
    call_in(Session::detect(), name, args)
}

/// Read-only capability report. Never captures the screen or moves input.
pub fn diagnostics() -> Value {
    let session = Session::detect();
    let platform = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let input = match input_allowed(session) {
        Ok(()) => json!({"available": true, "backend": "enigo"}),
        Err(reason) => json!({"available": false, "backend": "none", "reason": reason}),
    };
    let (monitors_json, monitor_error) = match monitors() {
        Ok(list) => (
            list.iter()
                .enumerate()
                .map(|(index, mon)| {
                    json!({"index": index, "x": mon.x, "y": mon.y, "width": mon.width,
                        "height": mon.height, "scaleFactor": mon.scale, "primary": mon.primary})
                })
                .collect::<Vec<_>>(),
            Value::Null,
        ),
        Err(error) => (Vec::new(), json!(error)),
    };
    json!({
        "platform": platform,
        "session": session.name(),
        "input": input,
        "capture": {"backend": "xcap", "monitors": monitors_json, "error": monitor_error},
        "engines": {
            "input": {"name": "enigo", "version": "0.6.1", "license": "MIT", "repository": "https://github.com/enigo-rs/enigo"},
            "capture": {"name": "xcap", "version": "0.9.8", "license": "MIT", "repository": "https://github.com/nashaofu/xcap"}
        },
        "coordinateSpace": "Primary-monitor physical pixels; (0,0) is the primary monitor's top-left corner and matches computer_screenshot output.",
        "tools": TOOL_NAMES,
    })
}

const TOOL_NAMES: &[&str] = &[
    "computer_screenshot",
    "computer_position",
    "computer_move",
    "computer_click",
    "computer_type",
    "computer_key",
    "computer_scroll",
];

fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

/// Model-visible definitions for the desktop automation surface. Registered
/// by the caller through the normal tool path — approval, audit, and
/// bounding apply like any other tool. None of these are trusted reads.
pub fn tool_definitions() -> Vec<crate::connectors::ToolView> {
    let point = json!({
        "x": {"type": "integer", "minimum": 0, "maximum": MAX_MONITOR_EDGE, "description": "X coordinate in primary-monitor physical pixels (0 = left edge)."},
        "y": {"type": "integer", "minimum": 0, "maximum": MAX_MONITOR_EDGE, "description": "Y coordinate in primary-monitor physical pixels (0 = top edge)."}
    });
    vec![
        crate::connectors::ToolView {
            name: "computer_screenshot".into(),
            description: "Capture the primary monitor as PNG. The image may be downscaled to a 1600px longest edge; the reported width/height are the true monitor pixels and define the coordinate space for clicks, moves, and scrolls.".into(),
            input_schema: schema(json!({}), &[]),
        },
        crate::connectors::ToolView {
            name: "computer_position".into(),
            description: "Report the cursor position in primary-monitor coordinates plus the monitor bounds. Read-only.".into(),
            input_schema: schema(json!({}), &[]),
        },
        crate::connectors::ToolView {
            name: "computer_move".into(),
            description: "Move the pointer to (x, y) in primary-monitor pixels without clicking.".into(),
            input_schema: schema(point.clone(), &["x", "y"]),
        },
        crate::connectors::ToolView {
            name: "computer_click".into(),
            description: "Move the pointer to (x, y) in primary-monitor pixels and click. button: left/right/middle (default left). doubleClick performs two clicks.".into(),
            input_schema: schema({
                let mut props = point.clone();
                props.as_object_mut().unwrap().insert("button".into(), json!({"type": "string", "enum": ["left", "right", "middle"], "default": "left"}));
                props.as_object_mut().unwrap().insert("doubleClick".into(), json!({"type": "boolean", "default": false}));
                props
            }, &["x", "y"]),
        },
        crate::connectors::ToolView {
            name: "computer_type".into(),
            description: "Type Unicode text into the focused application (up to 4096 bytes). This types characters, not shortcuts — use computer_key for keys.".into(),
            input_schema: schema(json!({"text": {"type": "string", "maxLength": MAX_TEXT_CHARS, "description": "Text to type literally."}}), &["text"]),
        },
        crate::connectors::ToolView {
            name: "computer_key".into(),
            description: "Press one key combo such as \"enter\", \"ctrl+c\", or \"alt+f4\". Modifiers: ctrl, alt, shift, meta. Keys: enter, tab, esc, space, backspace, delete, insert, home, end, pageup, pagedown, up/down/left/right, f1-f12, or a single character. All keys are released before returning.".into(),
            input_schema: schema(json!({"keys": {"type": "string", "maxLength": 64, "description": "Combo like \"ctrl+shift+t\"."}}), &["keys"]),
        },
        crate::connectors::ToolView {
            name: "computer_scroll".into(),
            description: "Scroll the mouse wheel, optionally after moving to (x, y). dy positive scrolls down, negative up; dx positive scrolls right, negative left (±240 steps each).".into(),
            input_schema: schema(json!({
                "dx": {"type": "integer", "minimum": -MAX_SCROLL_STEPS, "maximum": MAX_SCROLL_STEPS, "default": 0},
                "dy": {"type": "integer", "minimum": -MAX_SCROLL_STEPS, "maximum": MAX_SCROLL_STEPS},
                "x": {"type": "integer", "minimum": 0, "maximum": MAX_MONITOR_EDGE},
                "y": {"type": "integer", "minimum": 0, "maximum": MAX_MONITOR_EDGE}
            }), &["dy"]),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn definitions_are_strict_bounded_and_complete() {
        let defs = tool_definitions();
        assert_eq!(defs.len(), TOOL_NAMES.len());
        let mut seen = std::collections::HashSet::new();
        for def in &defs {
            assert!(seen.insert(def.name.clone()));
            assert!(TOOL_NAMES.contains(&def.name.as_str()));
            let schema = &def.input_schema;
            assert_eq!(schema["additionalProperties"], Value::Bool(false));
            assert!(!def.description.is_empty());
        }
        let click = defs.iter().find(|d| d.name == "computer_click").unwrap();
        assert_eq!(click.input_schema["required"], json!(["x", "y"]));
        let key = defs.iter().find(|d| d.name == "computer_key").unwrap();
        assert_eq!(key.input_schema["required"], json!(["keys"]));
    }

    #[test]
    fn key_combos_parse_and_reject_garbage() {
        let (mods, key) = parse_key_combo("ctrl+c").unwrap();
        assert_eq!(mods, vec![Key::Control]);
        assert_eq!(key, Key::Unicode('c'));
        let (mods, key) = parse_key_combo("Ctrl+Shift+T").unwrap();
        assert_eq!(mods, vec![Key::Control, Key::Shift]);
        assert_eq!(key, Key::Unicode('t'));
        assert_eq!(parse_key_combo("enter").unwrap(), (vec![], Key::Return));
        assert_eq!(
            parse_key_combo("alt+f4").unwrap(),
            (vec![Key::Alt], Key::F4)
        );
        assert_eq!(
            parse_key_combo("ctrl++").unwrap(),
            (vec![Key::Control], Key::Unicode('+'))
        );
        assert!(parse_key_combo("").is_err());
        assert!(parse_key_combo("ctrl+").is_err());
        assert!(parse_key_combo("ctrl+alt").is_err());
        assert!(parse_key_combo("a+b").is_err());
        assert!(parse_key_combo("ctrl+ctrl+c").is_err());
        assert!(parse_key_combo("notakey").is_err());
        assert!(parse_key_combo("ctrl+a+b+c+d+e").is_err());
    }

    #[test]
    fn coordinates_must_fit_the_primary_monitor() {
        let mon = Mon {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
            scale: 1.0,
            primary: true,
        };
        assert!(check_point(0, 0, &mon).is_ok());
        assert!(check_point(1919, 1079, &mon).is_ok());
        assert!(check_point(1920, 1080, &mon).is_err());
        assert!(check_point(-1, 5, &mon).is_err());
        assert!(check_point(5000, 5, &mon).is_err());
    }

    #[test]
    fn strict_validation_runs_before_any_desktop_contact() {
        // Unknown tool and malformed/unknown args fail validation whether or
        // not a desktop session exists.
        assert!(call("computer_hack", json!({})).is_err());
        assert!(call("computer_click", json!({"x": -5, "y": 10})).is_err());
        assert!(call("computer_click", json!({"x": 10, "y": 10, "surprise": 1})).is_err());
        assert!(call("computer_type", json!({"text": 42})).is_err());
        assert!(call("computer_scroll", json!({"dy": 10_000})).is_err());
        assert!(call("computer_scroll", json!({"dy": 3, "x": 10})).is_err());
        assert!(call("computer_key", json!({"keys": "ctrl+"})).is_err());
    }

    #[test]
    fn wayland_input_is_rejected_explicitly() {
        let err = call_in(
            Session::Wayland,
            "computer_click",
            &json!({"x": 1, "y": 1}).as_object().unwrap().clone(),
        );
        assert!(matches!(err, Err(e) if e.contains("Wayland")));
        let err = call_in(
            Session::Wayland,
            "computer_type",
            &json!({"text": "hi"}).as_object().unwrap().clone(),
        );
        assert!(matches!(err, Err(e) if e.contains("Wayland")));
        let err = call_in(Session::Wayland, "computer_position", &Map::new());
        assert!(matches!(err, Err(e) if e.contains("Wayland")));
        let err = call_in(
            Session::Headless,
            "computer_click",
            &json!({"x": 1, "y": 1}).as_object().unwrap().clone(),
        );
        assert!(matches!(err, Err(e) if e.contains("graphical desktop")));
    }

    #[test]
    fn diagnostics_never_touches_input_or_captures() {
        let report = diagnostics();
        assert!(report["platform"].is_string());
        assert!(report["session"].is_string());
        assert!(report["input"]["available"].is_boolean());
        assert!(report["engines"]["input"]["version"] == "0.6.1");
        assert!(report["engines"]["capture"]["version"] == "0.9.8");
        // Wayland sessions must report input unavailable with the honest reason.
        if report["session"] == "wayland" {
            assert_eq!(report["input"]["available"], false);
            assert!(report["input"]["reason"]
                .as_str()
                .unwrap()
                .contains("Wayland"));
        }
    }

    #[test]
    fn scaled_fit_preserves_aspect() {
        assert_eq!(scaled_fit(3840, 2160, 1600), (1600, 900));
        assert_eq!(scaled_fit(1000, 2000, 1600), (800, 1600));
        assert_eq!(scaled_fit(100, 100, 1600), (100, 100));
    }
    #[test]
    #[ignore = "Run with node scripts/test-desktop-automation.mjs; never on the user's desktop"]
    fn isolated_x11_driver_controls_fixture() {
        use std::{
            io::{BufRead, BufReader},
            process::{Command, Stdio},
            sync::mpsc,
            time::Duration,
        };
        assert_eq!(
            std::env::var("LOCALLM_DESKTOP_TEST_ISOLATED").as_deref(),
            Ok("1"),
            "Use the isolated Xvfb launcher"
        );
        assert_eq!(Session::detect(), Session::X11);
        struct Fixture(std::process::Child);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let mut fixture = Fixture(
            Command::new(std::env::var("LOCALLM_DESKTOP_TEST_FIXTURE").unwrap())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let stdout = fixture.0.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), "ready");
        call("computer_move", json!({"x":110,"y":120})).unwrap();
        match call("computer_position", json!({})).unwrap() {
            ComputerOutput::Json(value) => {
                assert_eq!(value["x"], 110);
                assert_eq!(value["y"], 120);
            }
            _ => panic!("position must be JSON"),
        }
        call("computer_click", json!({"x":110,"y":120})).unwrap();
        call("computer_type", json!({"text":"locallm"})).unwrap();
        call("computer_key", json!({"keys":"ctrl+a"})).unwrap();
        call("computer_scroll", json!({"dy":2})).unwrap();
        let mut letters = String::new();
        let mut clicked = false;
        let mut combo = false;
        let mut scrolled = false;
        for _ in 0..40 {
            let event = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("fixture did not receive input");
            let fields: Vec<_> = event.split_whitespace().collect();
            if fields[0] == "key" {
                let key = fields[1].parse::<u32>().unwrap();
                let modifiers = fields[2].parse::<u32>().unwrap();
                if key == 97 && modifiers & 4 != 0 {
                    combo = true;
                }
                if modifiers & 4 == 0 && (97..=122).contains(&key) {
                    letters.push(char::from_u32(key).unwrap());
                }
            }
            if event.starts_with("button 1 110 120") {
                clicked = true;
            }
            if event.starts_with("button 5 ") {
                scrolled = true;
            }
            if clicked && combo && scrolled && letters == "locallm" {
                break;
            }
        }
        assert!(clicked && combo && scrolled);
        assert_eq!(letters, "locallm");
        match call("computer_screenshot", json!({})).unwrap() {
            ComputerOutput::Screenshot { png, width, height } => {
                assert_eq!((width, height), (800, 600));
                let image = image::load_from_memory(&png).unwrap();
                assert_eq!((image.width(), image.height()), (800, 600));
            }
            _ => panic!("capture must return a PNG"),
        }
    }
}
