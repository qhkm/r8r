//! SSRF protection helpers shared across HTTP node and notification channels.
//!
//! Blocks access to localhost, private IP ranges, and non-http(s) schemes.
//! Disable with `R8R_ALLOW_INTERNAL_URLS=true` (useful for local development).

use std::net::IpAddr;

/// Returns true unless `R8R_ALLOW_INTERNAL_URLS=true` is set.
pub fn is_ssrf_protection_enabled() -> bool {
    std::env::var("R8R_ALLOW_INTERNAL_URLS")
        .map(|v| v.to_lowercase() != "true")
        .unwrap_or(true)
}

/// Validate a URL for SSRF safety. Returns an error message on rejection.
///
/// Blocks:
/// - Non-http/https schemes
/// - localhost / 127.x / ::1 / 0.0.0.0
/// - Private IP ranges (10.x, 172.16–31.x, 192.168.x)
/// - Link-local (169.254.x, cloud metadata)
/// - Common internal hostnames (*.local, *.internal, etc.)
pub fn check_ssrf(url: &str) -> Result<(), String> {
    if !is_ssrf_protection_enabled() {
        return Ok(());
    }

    let parsed = reqwest::Url::parse(url).map_err(|e| format!("Invalid URL '{}': {}", url, e))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "Unsupported URL scheme '{}'. Only http and https are allowed.",
                scheme
            ));
        }
    }

    if let Some(host) = parsed.host_str() {
        let host_lower = host.to_lowercase();

        if host_lower == "localhost"
            || host_lower == "127.0.0.1"
            || host_lower == "::1"
            || host_lower == "[::1]"
            || host_lower == "0.0.0.0"
        {
            return Err("Access to localhost is not allowed for security reasons.".to_string());
        }

        if let Ok(ip) = host.parse::<IpAddr>() {
            if is_private_or_special_ip(&ip) {
                return Err(
                    "Access to private or internal IP addresses is not allowed for security reasons.".to_string(),
                );
            }
        }

        if host_lower.ends_with(".local")
            || host_lower.ends_with(".internal")
            || host_lower.ends_with(".localhost")
            || host_lower == "metadata.google.internal"
            || host_lower == "169.254.169.254"
        {
            return Err(
                "Access to internal hostnames is not allowed for security reasons.".to_string(),
            );
        }
    }

    Ok(())
}

/// Returns true if the IP is private, loopback, link-local, or otherwise reserved.
pub fn is_private_or_special_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_unspecified()
                || (ipv4.octets()[0] == 100 && (ipv4.octets()[1] & 0xc0) == 64) // 100.64/10 CGNAT
        }
        IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6
                    .to_ipv4_mapped()
                    .map(|v4| is_private_or_special_ip(&IpAddr::V4(v4)))
                    .unwrap_or(false)
        }
    }
}
