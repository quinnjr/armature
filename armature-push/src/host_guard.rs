//! Shared SSRF / transport-safety host checks.
//!
//! These live outside the provider modules because all three providers need
//! them: Web Push validates an attacker-supplied subscription endpoint, while
//! FCM and APNS validate their (test-overridable) API base URLs so a
//! credential-bearing request can never be sent in the clear against a
//! non-loopback host.

#[cfg(feature = "web-push")]
use std::net::{Ipv4Addr, Ipv6Addr};

/// True for hosts that name the local machine.
///
/// Loopback is exempt from the https + internal-target checks, but only when
/// the caller has explicitly opted in (see `allow_insecure_loopback` on each
/// provider config) — the exemption is for local stub/integration tests and
/// must not be reachable in a production binary.
pub(crate) fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// True for IPv4 addresses that belong to internal/infrastructure ranges.
///
/// Covers RFC 1918 private space (10/8, 172.16/12, 192.168/16), link-local
/// (169.254/16 — the cloud metadata range), the unspecified address,
/// 100.64/10 (RFC 6598 CGNAT, which is routable to infrastructure inside many
/// cloud VPCs) and 192.0.0.0/24 (RFC 6890 IETF protocol assignments).
///
/// Loopback is deliberately *not* included here; callers decide whether to
/// exempt it, because it is the one range local tests legitimately need.
///
/// Only Web Push validates arbitrary caller-supplied hosts; FCM and APNS just
/// need the loopback check above for their base-URL scheme rule.
#[cfg(feature = "web-push")]
pub(crate) fn is_internal_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
    let protocol_assignments = o[0] == 192 && o[1] == 0 && o[2] == 0;

    ip.is_private() || ip.is_link_local() || ip.is_unspecified() || cgnat || protocol_assignments
}

/// True for IPv6 addresses that belong to internal ranges.
///
/// Critically, this resolves IPv4-mapped addresses (`::ffff:a.b.c.d`) first
/// and delegates to the IPv4 rules. Without that step
/// `https://[::ffff:169.254.169.254]/` reached the cloud metadata service:
/// segment 0 of a mapped address is `0`, so none of the IPv6 prefix masks
/// match, `Ipv6Addr::is_loopback` is false for `::ffff:127.0.0.1`, and the
/// `Host::Ipv4` arm is never consulted because the URL parses as `Host::Ipv6`.
#[cfg(feature = "web-push")]
pub(crate) fn is_internal_v6(ip: &Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_internal_v4(&v4) || v4.is_loopback();
    }

    let seg0 = ip.segments()[0];
    // fe80::/10 link-local or fc00::/7 unique-local, plus the unspecified address.
    ip.is_unspecified() || (seg0 & 0xffc0) == 0xfe80 || (seg0 & 0xfe00) == 0xfc00
}

#[cfg(all(test, feature = "web-push"))]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn v6(s: &str) -> Ipv6Addr {
        Ipv6Addr::from_str(s).unwrap()
    }

    fn v4(s: &str) -> Ipv4Addr {
        Ipv4Addr::from_str(s).unwrap()
    }

    #[test]
    fn ipv4_mapped_metadata_address_is_internal() {
        // The bypass this guard was written for: the plain form was blocked,
        // the mapped form reached the metadata service.
        assert!(is_internal_v6(&v6("::ffff:169.254.169.254")));
    }

    #[test]
    fn ipv4_mapped_private_and_loopback_are_internal() {
        assert!(is_internal_v6(&v6("::ffff:10.0.0.1")));
        assert!(is_internal_v6(&v6("::ffff:127.0.0.1")));
        assert!(is_internal_v6(&v6("::ffff:192.168.1.1")));
    }

    #[test]
    fn cgnat_range_is_internal() {
        assert!(is_internal_v4(&v4("100.64.0.1")));
        assert!(is_internal_v4(&v4("100.100.50.1")));
        assert!(is_internal_v4(&v4("100.127.255.255")));
        // Boundaries: 100.63.x and 100.128.x are outside 100.64.0.0/10.
        assert!(!is_internal_v4(&v4("100.63.255.255")));
        assert!(!is_internal_v4(&v4("100.128.0.0")));
    }

    #[test]
    fn protocol_assignment_range_is_internal() {
        assert!(is_internal_v4(&v4("192.0.0.1")));
        assert!(!is_internal_v4(&v4("192.0.1.1")));
    }

    #[test]
    fn public_addresses_are_not_internal() {
        assert!(!is_internal_v4(&v4("93.184.216.34")));
        assert!(!is_internal_v6(&v6("2606:2800:220:1:248:1893:25c8:1946")));
        assert!(!is_internal_v6(&v6("::ffff:93.184.216.34")));
    }

    #[test]
    fn native_ipv6_internal_ranges_still_blocked() {
        assert!(is_internal_v6(&v6("fe80::1")));
        assert!(is_internal_v6(&v6("fc00::1")));
        assert!(is_internal_v6(&v6("fd12:3456::1")));
        assert!(is_internal_v6(&v6("::")));
    }
}
