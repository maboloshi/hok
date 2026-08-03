//! Package identity — bucket/name splitting and extraction.
//!
//! Helpers that interpret a user-supplied query or dependency string of the
//! form `bucket/name` (or `bucket\name`) and extract the package name part.
//! Shared by `query`, `resolve`, `sync_install`, and `sync_remove`.

/// Split a bucket-qualified query ("bucket/name" or "bucket\name") into the
/// bucket prefix and the package name.
///
/// A backslash is only treated as a separator when it is not the leading
/// character, so regex escapes like `\d` keep their meaning in non-explicit
/// mode.
pub(crate) fn split_bucket_query(query: &str) -> (Option<String>, &str) {
    if let Some((bucket, name)) = query.split_once('/') {
        return (Some(bucket.to_owned()), name);
    }
    if let Some(pos) = query.find('\\') {
        if pos > 0 {
            return (Some(query[..pos].to_owned()), &query[pos + 1..]);
        }
    }
    (None, query)
}

/// Extact `name` from `bucket/name`.
pub(crate) fn extract_name<S: AsRef<str>>(input: S) -> String {
    input
        .as_ref()
        .split_once('/')
        .map(|(_, n)| n)
        .unwrap_or(input.as_ref())
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_bucket_query_handles_both_separators() {
        assert_eq!(
            split_bucket_query("extras/curl"),
            (Some("extras".to_owned()), "curl")
        );
        assert_eq!(
            split_bucket_query("extras\\curl"),
            (Some("extras".to_owned()), "curl")
        );
        assert_eq!(split_bucket_query("curl"), (None, "curl"));
        assert_eq!(split_bucket_query("\\d"), (None, "\\d"));
    }

    #[test]
    fn extract_name_strips_bucket_prefix() {
        assert_eq!(extract_name("extras/curl"), "curl");
        assert_eq!(extract_name("curl"), "curl");
    }
}
