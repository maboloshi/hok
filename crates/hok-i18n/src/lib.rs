//! In-house internationalisation runtime for hok.
//!
//! Replaces the `rust-i18n` crate: the locale files are read once at **build
//! time** by `build.rs` (see that file for the flattening rules) and baked into
//! a static table, so the release binary carries no YAML parser, no glob
//! walker, and no `serde_yaml`.
//!
//! # Behaviour (kept bit-for-bit compatible with the previous implementation)
//!
//! - **A missing key renders as the key itself** — never an empty string, never
//!   a panic. Some call sites rely on this: e.g.
//!   `crates/hok-i18n-derive` probes a secondary key and falls back when the
//!   lookup comes back unchanged.
//! - **Locale fallback walks the subtag chain** (`zh-Hant-CN` → `zh-Hant` →
//!   `zh`), matching BCP 47 truncation. There is deliberately **no** implicit
//!   `en` fallback: the previous backend had none either (its `fallback` option
//!   was never configured), and `tests/i18n_keys.rs` already guarantees that
//!   every key of `en.yml` also exists in `zh.yml`.
//! - **Interpolation** uses `%{name}` placeholders, substituted only for the
//!   names actually passed to [`t!`]; an unmatched placeholder stays verbatim,
//!   and a `t!` call without arguments performs no substitution at all.
//!
//! # Usage
//!
//! ```ignore
//! hok_i18n::set_locale("zh");
//! let text = hok_i18n::t!("output.done");                    // dynamic keys too
//! let text = hok_i18n::t!("error.bucket_not_found", name = "main");
//! ```

use std::sync::atomic::{AtomicUsize, Ordering};

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// `CURRENT_LOCALE` value used when the selected locale has no table; every
/// lookup then misses and [`translate`] returns the key.
const UNKNOWN_LOCALE: usize = usize::MAX;

/// Index of the currently selected locale inside [`LOCALES`].
static CURRENT_LOCALE: AtomicUsize = AtomicUsize::new(0);

/// Every locale that has a table, sorted by name.
pub fn available_locales() -> impl Iterator<Item = &'static str> {
    LOCALES.iter().map(|(name, _)| *name)
}

/// Select the locale used by [`translate`].
///
/// The value is matched against the generated tables through
/// [RFC 4647]'s lookup scheme: an exact table wins, otherwise progressively
/// shorter subtag prefixes are tried (`zh-Hant-CN` → `zh-Hant` → `zh`). An
/// unknown locale is accepted silently: lookups on it miss and render the key
/// itself, which is what the caller would have seen for a missing key anyway.
///
/// [RFC 4647]: https://datatracker.ietf.org/doc/html/rfc4647#section-3.4
pub fn set_locale(locale: &str) {
    CURRENT_LOCALE.store(resolve_locale(locale), Ordering::Relaxed);
}

/// Render `key` in the current locale without interpolating anything.
///
/// Returns the key unchanged when it is missing.
pub fn translate(key: impl AsRef<str>) -> String {
    let key = key.as_ref();
    lookup_current(key).map_or_else(|| key.to_string(), str::to_string)
}

/// Render `key` in the current locale, replacing `%{name}` placeholders.
///
/// `names[i]` is substituted with `values[i]`; the first match wins, exactly as
/// in the previous implementation.
pub fn translate_with(key: impl AsRef<str>, names: &[&str], values: &[String]) -> String {
    let key = key.as_ref();
    let template = lookup_current(key).unwrap_or(key);
    replace_patterns(template, names, values)
}

/// Resolve `key` in the current locale, or `None` when it has no translation.
pub fn lookup_current(key: &str) -> Option<&'static str> {
    table(CURRENT_LOCALE.load(Ordering::Relaxed)).and_then(|entries| binary_lookup(entries, key))
}

/// Index of the table [`set_locale`] should select for `locale`.
///
/// The subtag chain is walked here, once per `set_locale`, instead of on every
/// lookup. That keeps the runtime a plain index — no locale string to own, no
/// lock on the hot path.
///
/// The one behaviour this cannot express: rust-i18n walked the chain *per key*,
/// so a key missing from a more specific table fell through to the next shorter
/// tag. That needs two tags of the same chain to have tables (say `zh-CN.yml`
/// *and* `zh.yml`), which the shipped `en`/`zh` tables never do — so shipping a
/// second table for one language would silently change resolution, and this is
/// the function to revisit before doing so. The migration itself was verified
/// byte-for-byte against the removed reference dump (`cb34afb`).
fn resolve_locale(locale: &str) -> usize {
    let mut candidate = locale;
    loop {
        if let Some(index) = index_of(candidate) {
            return index;
        }
        // `trim_end_matches("-x")` mirrors rust-i18n: a private-use suffix
        // (`zh-x-foo`) is dropped rather than treated as a subtag.
        match candidate.rfind('-') {
            Some(end) => candidate = candidate[..end].trim_end_matches("-x"),
            None => return UNKNOWN_LOCALE,
        }
    }
}

fn index_of(locale: &str) -> Option<usize> {
    LOCALES.iter().position(|(name, _)| *name == locale)
}

fn table(index: usize) -> Option<&'static [(&'static str, &'static str)]> {
    LOCALES.get(index).map(|(_, table)| *table)
}

fn binary_lookup(
    entries: &'static [(&'static str, &'static str)],
    key: &str,
) -> Option<&'static str> {
    entries
        .binary_search_by(|(candidate, _)| (*candidate).cmp(key))
        .ok()
        .map(|index| entries[index].1)
}

/// Substitute `%{name}` placeholders, leaving unmatched ones untouched.
///
/// A faithful port of the previous implementation, including its scanning
/// quirk: a `%` starts a placeholder and the *next* `{` opens it, so
/// `%ab{name}` is treated as one placeholder named `name`.
fn replace_patterns(input: &str, patterns: &[&str], values: &[String]) -> String {
    const OPENING: u8 = b'{';
    const CLOSING: u8 = b'}';
    const SIGIL: u8 = b'%';
    const NONE: u8 = 0;
    const SEEN_SIGIL: u8 = 1;
    const SEEN_OPENING: u8 = 2;

    let bytes = input.as_bytes();
    let mut delimiters: Vec<usize> = Vec::new();
    let mut stage = NONE;
    for (i, &byte) in bytes.iter().enumerate() {
        match (stage, byte) {
            (SEEN_SIGIL, OPENING) => {
                stage = SEEN_OPENING;
                delimiters.push(i);
            }
            (SEEN_OPENING, CLOSING) => {
                stage = NONE;
                delimiters.push(i);
            }
            (_, SIGIL) => stage = SEEN_SIGIL,
            _ => {}
        }
    }

    let mut output = String::with_capacity(input.len() + 128);
    let mut copied_until = 0;
    for [opening, closing] in delimiters.as_chunks::<2>().0 {
        let (opening, closing) = (*opening, *closing);
        let name = &input[opening + 1..closing];
        if copied_until < opening {
            output.push_str(&input[copied_until..opening - 1]);
        }
        match patterns
            .iter()
            .zip(values)
            .find(|(pattern, _)| pattern.as_bytes() == name.as_bytes())
        {
            Some((_, value)) => output.push_str(value),
            None => output.push_str(&input[opening - 1..closing + 1]),
        }
        copied_until = closing + 1;
    }
    if copied_until < input.len() {
        output.push_str(&input[copied_until..]);
    }
    output
}

/// Render a translation, interpolating the `name = value` pairs.
///
/// Prefer the [`t!`] macro, which builds `names`/`values` from the call site.
#[doc(hidden)]
#[macro_export]
macro_rules! __hok_i18n_t {
    ($key:expr $(,)?) => {
        $crate::translate($key)
    };
    ($key:expr, $($name:tt = $value:expr),+ $(,)?) => {
        $crate::translate_with(
            $key,
            &[$($crate::__hok_i18n_pattern_name!($name)),+],
            &[$(::std::format!("{}", $value)),+],
        )
    };
}

/// Render a translation in the currently selected locale.
///
/// ```ignore
/// hok_i18n::t!("output.done");
/// hok_i18n::t!("error.package_not_found", name = package);
/// hok_i18n::t!(dynamic_key);           // any `AsRef<str>` expression
/// ```
#[macro_export]
macro_rules! t {
    ($($tokens:tt)*) => {
        $crate::__hok_i18n_t!($($tokens)*)
    };
}

/// Turn an interpolation name into the `&str` the runtime matches against.
///
/// Accepts both the plain-identifier form (`name = ..`) used throughout hok and
/// the string-literal form (`"ref" = ..`) needed for names that are Rust
/// keywords.
#[doc(hidden)]
#[macro_export]
macro_rules! __hok_i18n_pattern_name {
    ($name:ident) => {
        ::std::stringify!($name)
    };
    ($name:literal) => {
        $name
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Assemble the `names`/`values` pair the way [`t!`] does at a call site.
    fn args(names: &[&'static str]) -> (Vec<&'static str>, Vec<String>) {
        let values = names.iter().map(|name| format!("<{name}>")).collect();
        (names.to_vec(), values)
    }

    #[test]
    fn replaces_every_matching_placeholder() {
        let (names, values) = args(&["name", "count"]);
        assert_eq!(
            replace_patterns("Got %{count} for '%{name}'", &names, &values),
            "Got <count> for '<name>'"
        );
    }

    #[test]
    fn keeps_unmatched_placeholders_verbatim() {
        let (names, values) = args(&["name"]);
        assert_eq!(
            replace_patterns("%{other} and %{name}", &names, &values),
            "%{other} and <name>"
        );
    }

    #[test]
    fn repeats_and_duplicates_are_all_substituted() {
        let (names, values) = args(&["a"]);
        assert_eq!(replace_patterns("%{a}%{a}", &names, &values), "<a><a>");
    }

    #[test]
    fn first_matching_name_wins() {
        let names = vec!["a", "a"];
        let values = vec!["first".to_string(), "second".to_string()];
        assert_eq!(replace_patterns("%{a}", &names, &values), "first");
    }

    #[test]
    fn a_sigil_opens_at_the_next_brace() {
        // Scanning quirk inherited from rust-i18n: `%` starts a placeholder and
        // the next `{` opens it, so the bytes between them are dropped (the `b`
        // here) while everything before the sigil is kept verbatim.
        let (names, values) = args(&["a"]);
        assert_eq!(replace_patterns("%ab{a}", &names, &values), "%a<a>");
    }

    #[test]
    fn an_unterminated_placeholder_is_left_alone() {
        let (names, values) = args(&["a"]);
        assert_eq!(
            replace_patterns("%{a and %{", &names, &values),
            "%{a and %{"
        );
    }

    #[test]
    fn empty_input_stays_empty() {
        let (names, values) = args(&["a"]);
        assert_eq!(replace_patterns("", &names, &values), "");
    }

    #[test]
    fn every_generated_table_is_key_sorted_and_non_empty() {
        for (name, table) in LOCALES {
            assert!(!table.is_empty(), "locale {name} has no entries");
            assert!(
                table.windows(2).all(|pair| pair[0].0 < pair[1].0),
                "locale {name} is not strictly key-sorted"
            );
        }
    }

    /// `tests/i18n_keys.rs` reconciles the YAML files; this reconciles the
    /// generated tables, so a key can never be translated in one language only.
    #[test]
    fn en_and_zh_expose_the_same_keys() {
        let keys = |locale: &str| -> Vec<&'static str> {
            let index = index_of(locale).unwrap_or_else(|| panic!("locale {locale} is missing"));
            table(index)
                .expect("table")
                .iter()
                .map(|(key, _)| *key)
                .collect()
        };
        assert_eq!(keys("en"), keys("zh"));
    }

    /// Locale resolution is pure apart from reading [`LOCALES`], so it is tested
    /// directly here; the process-wide `set_locale` path is exercised
    /// end-to-end by `tests/cli_help_i18n.rs`, which renders help in both
    /// shipped locales.
    #[test]
    fn resolve_locale_walks_the_subtag_chain() {
        assert_eq!(resolve_locale("en"), index_of("en").unwrap());
        assert_eq!(resolve_locale("zh"), index_of("zh").unwrap());

        // Subtag truncation stops at the first prefix that has a table.
        assert_eq!(resolve_locale("en-US"), index_of("en").unwrap());
        assert_eq!(resolve_locale("en-GB-x-private"), index_of("en").unwrap());
        assert_eq!(resolve_locale("zh-Hant-CN"), index_of("zh").unwrap());
        assert_eq!(resolve_locale("zh-CN"), index_of("zh").unwrap());

        // No prefix has a table: every lookup will miss.
        for unknown in ["fr", "", "-", "de-DE", "x-en"] {
            assert_eq!(resolve_locale(unknown), UNKNOWN_LOCALE, "{unknown:?}");
        }
    }
}
