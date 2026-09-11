//! Guards the localization of CLI help output.
//!
//! Help text is patched at runtime from `locales/*.yml` by
//! `hok-i18n-derive`, so a rename, a new argument or a command that groups
//! subcommands can silently regress to the English doc comment (or, worse,
//! to a raw `cli_help.*` key). This test renders the help of **every**
//! command path — discovered by walking the command tree of the real binary —
//! in both shipped locales and asserts that:
//!
//! 1. no rendering contains a raw `cli_help.` key (a missing yml entry);
//! 2. in the Chinese short help every option/positional description carries
//!    CJK text, because an English-only description means the argument was
//!    never patched at all (the bug that hid `shim -g/--global`: a command
//!    whose own args live next to its `#[command(subcommand)]` field);
//! 3. the Chinese long help does not fall back to the English description —
//!    most commands have no dedicated `long_about` key and must reuse their
//!    translated short `about` instead.
//!
//! Deliberately ASCII descriptions (syntax markers such as `[default: 5]` and
//! the positional placeholder itself) are skipped, as are the multi-line
//! examples some `long_about` blocks carry — those are covered by check 1.
//!
//! Cost control: each command path is rendered three times (`zh -h`,
//! `zh --help`, `en -h`) instead of once per check. The English long help is
//! not rendered at all because the two tables expose identical key sets (see
//! `tests/i18n_keys.rs` and `hok-i18n`'s own unit test), so a raw-key leak
//! cannot be locale-specific — and the long path is already covered by the
//! Chinese rendering. `en -h` is kept for the English description check 3
//! compares against.

use std::process::Command;

/// Run `<path...> <flag> --lang <lang>` and return help from stdout *and*
/// stderr (clap writes some help/errors to stderr).
fn help(path: &[String], flag: &str, lang: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_hok"))
        .args(path)
        .arg(flag)
        .arg("--lang")
        .arg(lang)
        .output()
        .expect("run hok");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    text
}

/// Subcommand names listed directly under the `Subcommands:` heading. Names
/// are indented by exactly two spaces; wrapped descriptions are indented far
/// more, so they are not mistaken for names.
fn subcommands(path: &[String]) -> Vec<String> {
    let text = help(path, "--help", "en");
    let mut names = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        if !in_section {
            if line.trim_end() == "Subcommands:" {
                in_section = true;
            }
            continue;
        }
        if line.trim().is_empty() {
            break;
        }
        if !line.starts_with("  ") || line.starts_with("   ") {
            continue;
        }
        if let Some(name) = line.split_whitespace().next() {
            names.push(name.to_string());
        }
    }
    names
}

/// Every command path of the CLI, breadth-first, including the root.
fn all_command_paths() -> Vec<Vec<String>> {
    let mut paths: Vec<Vec<String>> = vec![Vec::new()];
    let mut next = 0;
    while next < paths.len() {
        for sub in subcommands(&paths[next]) {
            // clap's auto-generated `help` subcommand only re-prints other
            // commands' help; it has no args of its own to translate.
            if sub == "help" {
                continue;
            }
            let mut path = paths[next].clone();
            path.push(sub);
            paths.push(path);
        }
        next += 1;
    }
    paths
}

/// CJK ideographs plus CJK punctuation / fullwidth forms, i.e. anything that
/// only a real translation can contribute.
fn is_cjk(c: char) -> bool {
    matches!(
        c as u32,
        0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0xFF00..=0xFFEF
    )
}

#[test]
fn cli_help_is_fully_localized() {
    let paths = all_command_paths();
    assert!(
        paths.len() > 20,
        "expected the full command tree, found only {} paths",
        paths.len()
    );

    let mut failures: Vec<String> = Vec::new();
    for path in &paths {
        let name = if path.is_empty() {
            "hok".to_string()
        } else {
            format!("hok {}", path.join(" "))
        };

        // The three renderings every check below works from.
        let zh_short = help(path, "-h", "zh");
        let zh_long = help(path, "--help", "zh");
        let en_short = help(path, "-h", "en");

        // 1. No raw translation key may ever be rendered.
        for (lang, flag, text) in [
            ("zh", "-h", &zh_short),
            ("zh", "--help", &zh_long),
            ("en", "-h", &en_short),
        ] {
            for line in text.lines() {
                if line.contains("cli_help.") {
                    failures.push(format!("{name} {flag} [{lang}]: raw key: {line:?}"));
                }
            }
        }

        // 2. Every option/positional description in the Chinese short help is
        //    translated (ASCII-only descriptions are the unpatched fallback).
        for line in zh_short.lines() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with('-') || trimmed.starts_with('<')) {
                continue;
            }
            if line.len() - trimmed.len() < 2 {
                continue;
            }
            let Some((_, rest)) = trimmed.split_once("  ") else {
                continue;
            };
            let desc = rest.trim();
            if desc.is_empty() || desc.starts_with('[') || desc.chars().any(is_cjk) {
                continue;
            }
            failures.push(format!(
                "{name} -h [zh]: untranslated description for {trimmed:?}: {desc:?}"
            ));
        }

        // 3. The Chinese long help must not show the English description.
        let english_about = en_short.lines().nth(1).map(str::trim).unwrap_or_default();
        if !english_about.is_empty() && zh_long.contains(english_about) {
            failures.push(format!(
                "{name} --help [zh]: English description leaked: {english_about:?}"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} CLI help localization gap(s):\n{}",
        failures.len(),
        failures.join("\n")
    );
}
