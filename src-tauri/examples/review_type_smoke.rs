//! Type only into the focused control on our isolated native test display.
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use enigo::{Axis, Coordinate, Direction, Enigo, Key, Keyboard, Mouse, Settings};
    if std::env::var("LOCALLM_DESKTOP_TEST_ISOLATED").as_deref() != Ok("1") {
        return Err("Isolated native review required.".into());
    }
    let text = std::env::args().nth(1).ok_or("Missing fixture text")?;
    let mut input = Enigo::new(&Settings::default())?;
    if std::env::args().nth(2).as_deref() == Some("scroll") {
        input.move_mouse(1000, 650, Coordinate::Abs)?;
        input.scroll(text.parse()?, Axis::Vertical)?;
        return Ok(());
    }
    if std::env::args().nth(2).as_deref() == Some("replace") {
        input.key(Key::Control, Direction::Press)?;
        input.key(Key::Unicode('a'), Direction::Click)?;
        input.key(Key::Control, Direction::Release)?;
    }
    input.text(&text)?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Linux isolated test only.");
}
