//! Read/capture only the native application on our isolated test display.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use enigo::{Axis, Button, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
    use std::{path::PathBuf, time::Duration};
    if std::env::var("LOCALLM_DESKTOP_TEST_ISOLATED").as_deref() != Ok("1") {
        return Err("Use scripts/native-linux-smoke.mjs on its isolated display.".into());
    }
    let output = PathBuf::from(std::env::args().nth(1).ok_or("Missing capture directory")?);
    let id = u64::from_str_radix(
        std::env::args()
            .nth(2)
            .ok_or("Missing isolated window id")?
            .trim_start_matches("0x"),
        16,
    )?;
    let monitor = xcap::Monitor::all()?
        .into_iter()
        .next()
        .ok_or("No isolated monitor")?;
    std::thread::sleep(Duration::from_secs(3));
    monitor
        .capture_image()?
        .save(output.join("native-home.png"))?;
    #[link(name = "X11")]
    unsafe extern "C" {
        fn XOpenDisplay(name: *const std::ffi::c_char) -> *mut std::ffi::c_void;
        fn XSetInputFocus(
            display: *mut std::ffi::c_void,
            focus: std::ffi::c_ulong,
            revert: std::ffi::c_int,
            time: std::ffi::c_ulong,
        ) -> std::ffi::c_int;
        fn XSync(display: *mut std::ffi::c_void, discard: std::ffi::c_int) -> std::ffi::c_int;
        fn XCloseDisplay(display: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
    unsafe {
        let display = XOpenDisplay(std::ptr::null());
        if display.is_null() {
            return Err("No isolated X11 display".into());
        }
        XSetInputFocus(display, id as std::ffi::c_ulong, 1, 0);
        XSync(display, 0);
        XCloseDisplay(display);
    }
    let mut input = Enigo::new(&Settings::default())?;
    let actions: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(std::env::var(
        "LOCALLM_NATIVE_REVIEW_ACTIONS",
    )?)?)?;
    for action in actions {
        if let Some(position) = action["click"].as_array() {
            input.move_mouse(
                position[0].as_i64().ok_or("x")? as i32,
                position[1].as_i64().ok_or("y")? as i32,
                Coordinate::Abs,
            )?;
            input.button(Button::Left, Direction::Click)?;
        }
        if let Some(scroll) = action["scroll"].as_i64() {
            input.scroll(scroll as i32, Axis::Vertical)?;
        }
        if let Some(text) = action["text"].as_str() {
            input.text(text)?;
        }
        if let Some(keys) = action["keys"].as_str() {
            match keys {
                "palette" => {
                    input.key(Key::Control, Direction::Press)?;
                    input.key(Key::Shift, Direction::Press)?;
                    input.key(Key::Unicode('p'), Direction::Click)?;
                    input.key(Key::Shift, Direction::Release)?;
                    input.key(Key::Control, Direction::Release)?;
                }
                "settings" => {
                    input.key(Key::Control, Direction::Press)?;
                    input.key(Key::Unicode(','), Direction::Click)?;
                    input.key(Key::Control, Direction::Release)?;
                }
                "tab" => input.key(Key::Tab, Direction::Click)?,
                "shift-tab" => {
                    input.key(Key::Shift, Direction::Press)?;
                    input.key(Key::Tab, Direction::Click)?;
                    input.key(Key::Shift, Direction::Release)?;
                }
                "location" => {
                    input.key(Key::Control, Direction::Press)?;
                    input.key(Key::Unicode('l'), Direction::Click)?;
                    input.key(Key::Control, Direction::Release)?;
                }
                "select-all" => {
                    input.key(Key::Control, Direction::Press)?;
                    input.key(Key::Unicode('a'), Direction::Click)?;
                    input.key(Key::Control, Direction::Release)?;
                }
                "enter" => input.key(Key::Return, Direction::Click)?,
                "escape" => input.key(Key::Escape, Direction::Click)?,
                _ => return Err("Unknown review key".into()),
            }
        }
        std::thread::sleep(Duration::from_millis(
            action["waitMs"].as_u64().unwrap_or(500).min(5000),
        ));
        if let Some(name) = action["capture"].as_str() {
            if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Err("Invalid capture name".into());
            }
            monitor
                .capture_image()?
                .save(output.join(format!("{name}.png")))?;
        }
    }
    println!("Native review actions completed on isolated X11 display.");
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This smoke capture requires Linux and the isolated Xvfb launcher.");
}
