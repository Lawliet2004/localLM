//! Read/capture only the native application on our isolated test display.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
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
    input.key(Key::Control, Direction::Press)?;
    input.key(Key::Unicode(','), Direction::Click)?;
    input.key(Key::Control, Direction::Release)?;
    std::thread::sleep(Duration::from_secs(1));
    monitor
        .capture_image()?
        .save(output.join("native-settings.png"))?;
    println!(
        "Native LocalLM desktop captured: {}×{}; settings shortcut sent on isolated X11 display.",
        monitor.width()?,
        monitor.height()?
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This smoke capture requires Linux and the isolated Xvfb launcher.");
}
