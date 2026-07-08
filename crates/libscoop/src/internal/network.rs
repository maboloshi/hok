use curl::easy::Easy;
use std::time::Duration;

/// Get the content length of a URL via HTTP HEAD.
pub fn get_content_length(url: &str, proxy: Option<&str>) -> Option<f64> {
    let mut easy = Easy::new();
    easy.get(true).ok()?;
    easy.url(url).ok()?;
    if let Some(proxy) = proxy {
        easy.proxy(proxy).ok()?;
    }
    easy.nobody(true).ok()?;
    easy.follow_location(true).ok()?;
    easy.connect_timeout(Duration::from_secs(30)).ok()?;
    easy.perform().ok()?;
    easy.content_length_download().ok()
}

/// Check if a URL returns a successful HTTP status (2xx or 3xx).
///
/// Returns `true` if the URL is accessible, `false` if server returns 4xx/5xx.
pub fn head_url(url: &str, proxy: Option<&str>, timeout_secs: u64) -> Result<bool, curl::Error> {
    let mut easy = Easy::new();
    easy.get(true)?;
    easy.url(url)?;
    easy.nobody(true)?;
    easy.follow_location(true)?;
    easy.timeout(Duration::from_secs(timeout_secs))?;
    if let Some(proxy) = proxy {
        easy.proxy(proxy)?;
    }
    easy.perform()?;
    let code = easy.response_code()?;
    Ok((200..400).contains(&code))
}

/// Download a URL's content as bytes via HTTP GET.
pub fn download_file(url: &str, proxy: Option<&str>) -> Result<Vec<u8>, curl::Error> {
    let mut easy = Easy::new();
    easy.get(true)?;
    easy.url(url)?;
    easy.follow_location(true)?;
    easy.timeout(Duration::from_secs(120))?;
    if let Some(proxy) = proxy {
        easy.proxy(proxy)?;
    }

    let mut data = Vec::new();
    {
        let mut transfer = easy.transfer();
        transfer.write_function(|buf| {
            data.extend_from_slice(buf);
            Ok(buf.len())
        })?;
        transfer.perform()?;
    }
    Ok(data)
}
