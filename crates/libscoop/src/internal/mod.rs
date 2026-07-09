pub mod archive;
pub mod dag;
pub mod env;
pub mod fs;
pub mod git;
pub mod manifest_cache;
pub mod network;
pub mod os;
pub mod path;

/// Compare two semantic version strings.
///
/// Supports:
/// - Numeric segments: `1.2.3` > `1.2.0`
/// - Pre-release suffixes: `1.0.0-rc1` < `1.0.0`
/// - Mixed text+number: `1.2.0-beta` > `1.2.0-alpha`
/// - `v`/`V` prefix stripped automatically
pub fn compare_versions<S: AsRef<str>>(ver_a: S, ver_b: S) -> std::cmp::Ordering {
    let a = ver_a.as_ref().trim_start_matches(|c| c == 'v' || c == 'V');
    let b = ver_b.as_ref().trim_start_matches(|c| c == 'v' || c == 'V');

    let a_parts: Vec<&str> = a.split(&['.', '-', '_', '+'][..]).collect();
    let b_parts: Vec<&str> = b.split(&['.', '-', '_', '+'][..]).collect();

    let max_len = a_parts.len().max(b_parts.len());

    for i in 0..max_len {
        let a_seg = a_parts.get(i).copied().unwrap_or("0");
        let b_seg = b_parts.get(i).copied().unwrap_or("0");

        // Empty segment means end of pre-release vs release
        if a_seg.is_empty() && b_seg.is_empty() {
            continue;
        }

        let a_is_num = a_seg.parse::<u64>().ok();
        let b_is_num = b_seg.parse::<u64>().ok();

        match (a_is_num, b_is_num) {
            (Some(a_num), Some(b_num)) => {
                if a_num != b_num {
                    return a_num.cmp(&b_num);
                }
            }
            (Some(_), None) => return std::cmp::Ordering::Greater, // num > text
            (None, Some(_)) => return std::cmp::Ordering::Less,    // text < num
            (None, None) => {
                match a_seg.cmp(b_seg) {
                    std::cmp::Ordering::Equal => continue,
                    other => return other,
                }
            }
        }
    }

    // All segments equal. If one version has more pre-release identifiers,
    // it's considered smaller (e.g., 1.0.0-rc1 < 1.0.0).
    // Detect by checking if the original string contained '-' or similar.
    let a_has_suffix = a.contains('-') || a.contains('_') || a.contains('+');
    let b_has_suffix = b.contains('-') || b.contains('_') || b.contains('+');

    match (a_has_suffix, b_has_suffix) {
        (true, false) => std::cmp::Ordering::Less,    // 1.0.0-rc < 1.0.0
        (false, true) => std::cmp::Ordering::Greater, // 1.0.0 > 1.0.0-rc
        _ => std::cmp::Ordering::Equal,
    }
}
