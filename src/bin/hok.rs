#[tokio::main]
pub async fn main() {
    use crossterm::ExecutableCommand;

    let code = hok::create_app().await;
    // On error paths the event-loop thread may not have been joined, so its
    // CursorGuard drop never ran and the terminal cursor stays hidden.
    // Restore it before exiting (best-effort; exit kills any stragglers).
    let _ = std::io::stdout().execute(crossterm::cursor::Show);
    std::process::exit(code);
}
