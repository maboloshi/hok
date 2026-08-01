//! URL string helpers.
//!
//! Provides utility functions for common parsing operations on URL strings, such as extracting the filename,
//! extracting the basename, and decoding percent-encoding.
//!
//! # Usage
//!
//! These functions are pure functions (no I/O, no network) and can be safely called from any thread.
//!
//! # Notes
//!
//! - [`remote_filename`] first percent-decodes the URL, then extracts the last path segment;
//!   unlike [`basename`], which does not decode and strips the extension.
//! - These functions do not validate the URL; invalid URLs typically return the entire input string.

/// Extracts the filename portion of a URL (the last path segment, percent-decoded).
///
/// # Example
///
/// ```
/// # use libscoop::internal::url::remote_filename;
/// assert_eq!(remote_filename("https://example.com/foo%20bar.zip"), "foo bar.zip");
/// assert_eq!(remote_filename("https://example.com/pkg/"), "");
/// ```
pub fn remote_filename(url: &str) -> String {
    let decoded = decoded(url);
    decoded.rsplit('/').next().unwrap_or(&decoded).to_string()
}

/// Extract the basename of the URL (the part after removing the file extension, without percent-decoding).
///
/// # Example
///
/// ```
/// # use libscoop::internal::url::basename;
/// assert_eq!(basename("https://example.com/archive.tar.gz"), "archive.tar");
/// assert_eq!(basename("https://example.com/noext"), "noext");
/// ```
pub fn basename(url: &str) -> String {
    let filename = url.rsplit('/').next().unwrap_or(url);
    match filename.rfind('.') {
        Some(pos) => filename[..pos].to_string(),
        None => filename.to_string(),
    }
}

/// Decode URL percent-encoding, replacing `%XX` with the corresponding byte character.
///
/// For invalid `%XX` sequences (non-hexadecimal characters), the input character is preserved as-is.
///
/// # Example
///
/// ```
/// # use libscoop::internal::url::decoded;
/// assert_eq!(decoded("hello%20world"), "hello world");
/// assert_eq!(decoded("no_encoding"), "no_encoding");
/// ```
pub fn decoded(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let hex: String = chars.by_ref().take(2).collect();
            if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                result.push(byte as char);
                continue;
            }
            result.push('%');
            result.push_str(&hex);
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_filename_decodes_percent() {
        assert_eq!(remote_filename("https://example.com/foo%20bar.zip"), "foo bar.zip");
    }

    #[test]
    fn remote_filename_plain_url() {
        assert_eq!(remote_filename("https://example.com/releases/app-1.0.zip"), "app-1.0.zip");
    }

    #[test]
    fn basename_strips_extension() {
        assert_eq!(basename("https://example.com/archive.tar.gz"), "archive.tar");
    }

    #[test]
    fn basename_no_extension() {
        assert_eq!(basename("https://example.com/noext"), "noext");
    }

    #[test]
    fn decoded_percent_space() {
        assert_eq!(decoded("hello%20world"), "hello world");
    }

    #[test]
    fn decoded_no_encoding() {
        assert_eq!(decoded("plaintext"), "plaintext");
    }

    #[test]
    fn decoded_invalid_percent_kept() {
        // %ZZ is not valid hex, should be kept as-is
        assert!(decoded("%ZZ").contains('%'));
    }
}
