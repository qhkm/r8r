/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
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

        // `host_str()` keeps the brackets around IPv6 literals; strip them so
        // the address parses. (IPv4 literals arrive already normalized.)
        let literal = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = literal.parse::<IpAddr>() {
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
            let o = ipv4.octets();
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_broadcast()
                || ipv4.is_unspecified()
                || ipv4.is_multicast()
                || o[0] == 0 // 0.0.0.0/8 "this network"
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64/10 CGNAT
                || (o[0] == 198 && (o[1] & 0xfe) == 18) // 198.18/15 benchmarking
                || o[0] >= 240 // 240/4 reserved
        }
        IpAddr::V6(ipv6) => {
            let seg = ipv6.segments();
            // IPv4-mapped (::ffff:a.b.c.d), IPv4-compatible (::a.b.c.d) and
            // NAT64 (64:ff9b::a.b.c.d) addresses reach the embedded IPv4 host.
            let embedded_v4 = ipv6.to_ipv4_mapped().or_else(|| {
                let o = ipv6.octets();
                let v4 = std::net::Ipv4Addr::new(o[12], o[13], o[14], o[15]);
                if seg[..6] == [0, 0, 0, 0, 0, 0] || seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] {
                    Some(v4)
                } else {
                    None
                }
            });
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || ipv6.is_multicast()
                || (seg[0] & 0xfe00) == 0xfc00 // fc00::/7 unique local
                || (seg[0] & 0xffc0) == 0xfe80 // fe80::/10 link-local
                || (seg[0] & 0xffc0) == 0xfec0 // fec0::/10 site-local (deprecated)
                || embedded_v4
                    .map(|v4| is_private_or_special_ip(&IpAddr::V4(v4)))
                    .unwrap_or(false)
        }
    }
}

/// DNS resolver that drops private/internal addresses, so a public hostname
/// cannot be pointed (or rebound) at an internal service.
struct SsrfSafeResolver;

impl reqwest::dns::Resolve for SsrfSafeResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let host = name.as_str().to_string();
            let addrs: Vec<std::net::SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if !is_ssrf_protection_enabled() {
                return Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs);
            }
            let allowed: Vec<std::net::SocketAddr> = addrs
                .into_iter()
                .filter(|a| !is_private_or_special_ip(&a.ip()))
                .collect();
            if allowed.is_empty() {
                return Err(format!(
                    "Host '{}' resolves only to private or internal addresses; blocked for security reasons.",
                    host
                )
                .into());
            }
            Ok(Box::new(allowed.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Maximum redirects followed by SSRF-hardened clients (reqwest's default).
const MAX_REDIRECTS: usize = 10;

/// Redirect policy that re-runs [`check_ssrf`] on every redirect target.
fn ssrf_redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        match check_ssrf(attempt.url().as_str()) {
            Ok(()) => attempt.follow(),
            Err(msg) => attempt.error(format!("Redirect blocked: {}", msg)),
        }
    })
}

/// A `ClientBuilder` for requests to user-supplied URLs: every redirect hop is
/// re-validated and resolved addresses are filtered, in addition to the
/// up-front [`check_ssrf`] callers perform on the initial URL.
pub fn hardened_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .redirect(ssrf_redirect_policy())
        .dns_resolver(std::sync::Arc::new(SsrfSafeResolver))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(url: &str) -> bool {
        check_ssrf(url).is_err()
    }

    #[test]
    fn test_blocks_bracketed_ipv6_literals() {
        assert!(blocked("http://[::1]/"));
        assert!(blocked("http://[::ffff:127.0.0.1]/"));
        assert!(blocked("http://[::ffff:10.0.0.1]/"));
        assert!(blocked("http://[::ffff:169.254.169.254]/"));
        assert!(blocked("http://[fd00::1]/"));
        assert!(blocked("http://[fe80::1]/"));
        assert!(blocked("http://[64:ff9b::7f00:1]/"));
        assert!(blocked("http://[::]/"));
    }

    #[test]
    fn test_blocks_encoded_ipv4_literals() {
        // The URL parser normalizes these to 127.0.0.1 / 10.0.0.1.
        assert!(blocked("http://2130706433/"));
        assert!(blocked("http://0x7f.0.0.1/"));
        assert!(blocked("http://0177.0.0.1/"));
        assert!(blocked("http://10.1/"));
    }

    #[test]
    fn test_allows_public_addresses() {
        assert!(!blocked("https://example.com/"));
        assert!(!blocked("http://8.8.8.8/"));
        assert!(!blocked("http://[2606:4700:4700::1111]/"));
    }

    #[test]
    fn test_special_ipv4_ranges() {
        for ip in [
            "0.1.2.3",
            "100.64.0.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
        ] {
            assert!(is_private_or_special_ip(&ip.parse().unwrap()), "{ip}");
        }
        assert!(!is_private_or_special_ip(&"1.1.1.1".parse().unwrap()));
    }

    #[tokio::test]
    async fn test_redirect_to_internal_address_is_blocked() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 1024];
            let _ = sock.read(&mut buf).await;
            let _ = sock
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/latest/meta-data/\r\nContent-Length: 0\r\n\r\n",
                )
                .await;
        });

        // The listener itself is on loopback, so hit it directly rather than
        // through check_ssrf; the redirect hop is what is under test.
        let client = hardened_client_builder().build().unwrap();
        let err = client
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap_err();
        assert!(err.is_redirect(), "expected redirect error, got {err:?}");
    }
}
