use crossterm::{
    style::{Color, Print, SetForegroundColor},
    ExecutableCommand,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::{fmt::Display, io};

mod cmd;
mod cui;
mod output;
mod util;

type Result<T> = anyhow::Result<T>;

static DETAIL_MODE: AtomicBool = AtomicBool::new(false);

pub fn set_detail(enabled: bool) {
    DETAIL_MODE.store(enabled, Ordering::Relaxed);
}

pub fn is_detail() -> bool {
    DETAIL_MODE.load(Ordering::Relaxed)
}

fn error<T: Display>(input: &T) -> io::Result<()> {
    let mut stderr = io::stderr();
    stderr
        .execute(SetForegroundColor(Color::Red))?
        .execute(Print("ERROR "))?
        .execute(SetForegroundColor(Color::Reset))?
        .execute(Print(input))?
        .execute(Print("\n"))?;
    Ok(())
}

fn report(err: &anyhow::Error) {
    let _ = error(err);
    if let Some(cause) = err.source() {
        eprintln!("\nCaused by:");
        for (i, e) in std::iter::successors(Some(cause), |e| e.source()).enumerate() {
            eprintln!("   {}: {}", i, e);
        }
    }
}

pub fn create_app() -> bool {
    cmd::start().inspect_err(report).is_err()
}