#[tokio::main]
pub async fn main() {
    use crossterm::ExecutableCommand;
    use std::io::IsTerminal;

    let code = hok::create_app().await;
    // On error paths the event-loop thread may not have been joined, so its
    // CursorGuard drop never ran and the terminal cursor stays hidden.
    // Restore it before exiting (best-effort; exit kills any stragglers).
    //
    // Terminal only: the escape must never land in redirected stdout, where it
    // corrupts machine-readable output (`hok export > scoopfile.json` became
    // unparseable for Scoop's `ConvertFrom-Json` and for hok itself).
    if std::io::stdout().is_terminal() {
        let _ = std::io::stdout().execute(crossterm::cursor::Show);
    }
    std::process::exit(code);
}
