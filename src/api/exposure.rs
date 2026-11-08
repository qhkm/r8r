/*
 * Copyright: Kitakod Ventures 2026
 * This file and its contents are licensed under the AGPLv3 License.
 * Please see the included NOTICE for copyright information and
 * LICENSE-AGPL for a copy of the license.
 */
//! Startup policy for which addresses the server may listen on without auth.
//!
//! Anyone who can reach the API can create and run workflows, and workflows
//! can use stored credentials. So the server refuses to listen on a
//! non-loopback address unless `R8R_API_KEY` is set, or the operator opts out
//! explicitly with `--allow-unauthenticated`.

use std::net::SocketAddr;

/// Outcome of the exposure check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exposure {
    /// Loopback only, or reachable but protected by an API key.
    Safe,
    /// Reachable from the network with no API key, allowed by explicit opt-out.
    UnauthenticatedNetwork,
}

/// Decide whether the server may start on `addrs`.
///
/// Returns `Err` with an operator-facing message when the server would be
/// reachable from the network without an API key and no opt-out was given.
pub fn check_exposure(
    addrs: &[SocketAddr],
    api_key_set: bool,
    allow_unauthenticated: bool,
) -> Result<Exposure, String> {
    let network_reachable = addrs.iter().any(|a| !a.ip().is_loopback());
    if !network_reachable || api_key_set {
        return Ok(Exposure::Safe);
    }
    if allow_unauthenticated {
        return Ok(Exposure::UnauthenticatedNetwork);
    }
    let shown = addrs
        .first()
        .map(|a| a.to_string())
        .unwrap_or_else(|| "the requested address".to_string());
    Err(format!(
        "refusing to listen on {shown} without authentication.\n\
         \n\
         Anyone who can reach this address could create and run workflows and use stored credentials.\n\
         \n\
         Choose one:\n  \
         - set an API key:      export R8R_API_KEY=\"$(openssl rand -hex 32)\"\n  \
         - listen locally only: r8r server --host 127.0.0.1\n  \
         - accept the risk:     r8r server --allow-unauthenticated   (only on a trusted, isolated network)"
    ))
}

/// Read `R8R_API_KEY`, treating an empty value as unset.
pub fn api_key_from_env() -> Option<String> {
    std::env::var("R8R_API_KEY").ok().filter(|k| !k.is_empty())
}

/// Read `R8R_ALLOW_UNAUTHENTICATED` (`true`/`1`/`yes`, case-insensitive).
pub fn allow_unauthenticated_from_env() -> bool {
    std::env::var("R8R_ALLOW_UNAUTHENTICATED")
        .map(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    #[test]
    fn loopback_without_key_is_safe() {
        for a in ["127.0.0.1:8080", "[::1]:8080"] {
            assert_eq!(check_exposure(&[addr(a)], false, false), Ok(Exposure::Safe));
        }
    }

    #[test]
    fn all_interfaces_without_key_is_refused() {
        for a in ["0.0.0.0:8080", "[::]:8080", "192.168.1.10:8080"] {
            let err = check_exposure(&[addr(a)], false, false).unwrap_err();
            assert!(err.contains("R8R_API_KEY"), "{err}");
            assert!(err.contains(a), "{err}");
        }
    }

    #[test]
    fn network_with_key_is_safe() {
        assert_eq!(
            check_exposure(&[addr("0.0.0.0:8080")], true, false),
            Ok(Exposure::Safe)
        );
    }

    #[test]
    fn network_without_key_with_opt_out_is_flagged() {
        assert_eq!(
            check_exposure(&[addr("0.0.0.0:8080")], false, true),
            Ok(Exposure::UnauthenticatedNetwork)
        );
    }

    #[test]
    fn any_non_loopback_address_counts_as_exposed() {
        let addrs = [addr("127.0.0.1:8080"), addr("10.0.0.5:8080")];
        assert!(check_exposure(&addrs, false, false).is_err());
    }
}
