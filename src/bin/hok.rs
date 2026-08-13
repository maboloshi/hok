pub fn main() {
    use crossterm::ExecutableCommand;

    let ok = hok::create_app();
    // On error paths the event-loop thread may not have been joined, so its
    // CursorGuard drop never ran and the terminal cursor stays hidden.
    // Restore it before exiting (best-effort; exit kills any stragglers).
    let _ = std::io::stdout().execute(crossterm::cursor::Show);
    std::process::exit(ok as i32);
}
