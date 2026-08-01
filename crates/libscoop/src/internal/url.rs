//! URL string helpers.
//!
//! 提供对 URL 字符串进行常见解析操作的工具函数，例如提取文件名、
//! 提取 basename 以及解码 percent-encoding。
//!
//! # 使用说明
//!
//! 这些函数均为纯函数（无 I/O、无网络），可在任何线程中安全调用。
//!
//! # 注意事项
//!
//! - [`remote_filename`] 先对 URL 进行 percent-decode，再提取最后一段路径；
//!   与 [`basename`] 不同，后者不做解码、且会去掉扩展名。
//! - 这些函数不验证 URL 的合法性；无效 URL 通常会返回整个输入字符串。

/// 提取 URL 的文件名部分（最后一段路径，经 percent-decode 处理）。
///
/// # 示例
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

/// 提取 URL 的 basename（文件名去掉扩展名后的部分，不做 percent-decode）。
///
/// # 示例
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

/// 对 URL percent-encoding 进行解码，将 `%XX` 替换为对应字节字符。
///
/// 对于无效的 `%XX` 序列（非十六进制字符），原样保留输入字符。
///
/// # 示例
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
