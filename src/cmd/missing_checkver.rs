//! Report the `checkver` / `autoupdate` status of every manifest in a bucket.

use clap::Parser;
use libscoop::package::missing_checkver;
use std::path::PathBuf;

use crate::cmd::shared_args::validate_dir;
use crate::output::style;
use crate::{Result, output};

/// Check bucket manifests missing checkver and autoupdate
#[derive(Debug, Parser)]
pub struct Args {
    // Upstream `-App` is a positional parameter (`bin/missing-checkver.ps1:13`)
    // and every sibling bucket-scanning command exposes it the same way
    // (`checkurls`, `checkhashes`, `checkver`, `formatjson`), so this is a
    // positional too — not `-a/--app`.
    /// Specific app(s) to check (wildcards and .json optional, case-insensitive; default: all)
    #[arg(default_value = "*")]
    app: Vec<String>,

    /// Bucket directory to scan for manifests
    #[arg(short = 'd', long, default_value = ".")]
    dir: PathBuf,

    /// Skip manifests that have both checkver and autoupdate
    #[arg(short = 's', long, action = clap::ArgAction::SetTrue)]
    skip_supported: bool,
}

/// The `[C]heckver | [A]utoupdate` legend printed once before the table.
///
/// Verbatim from official `bin/missing-checkver.ps1:31-37`, including the
/// literal third line that marks the colour-key column — that line is
/// structural, not prose, so it is neither coloured nor translated.
fn print_legend() {
    println!("[{}]heckver", style::success("C"),);
    println!(" | [{}]utoupdate", style::accent("A"));
    println!(" |  |");
}

pub async fn execute(args: Args) -> Result<()> {
    validate_dir(&args.dir)?;
    let dir = &args.dir;

    let report = missing_checkver::scan(dir, &args.app)?;

    print_legend();
    for warning in &report.warnings {
        output::warn(warning);
    }
    for item in &report.items {
        if args.skip_supported && item.is_supported() {
            continue;
        }
        // `[C][A] name`: a present property is its letter, an absent one (or an
        // unparsable manifest) is a blank cell — same shape as the official
        // table, where a parse failure also renders `[ ][ ]`.
        print!(
            "[{}][{}] ",
            if item.checkver == Some(true) {
                style::success("C")
            } else {
                " ".to_string()
            },
            if item.autoupdate == Some(true) {
                style::accent("A")
            } else {
                " ".to_string()
            }
        );
        println!("{}", style::emphasis(&item.name));
    }

    output::info(hok_i18n::t!(
        "cmd.missing_checkver_scan",
        total = report.total,
        missing = report.missing_checkver,
        noauto = report.missing_autoupdate,
        unparsable = report.unparsable
    ));
    if report.missing_checkver + report.missing_autoupdate + report.unparsable == 0 {
        output::info(hok_i18n::t!("cmd.missing_checkver_all"));
    }

    Ok(())
}
