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
/// (169.254/16 — the cloud metadata range), the whole 0.0.0.0/8 "this network"
/// block, 100.64/10 (RFC 6598 CGNAT, which is routable to infrastructure
/// inside many cloud VPCs), 192.0.0.0/24 (RFC 6890 IETF protocol
/// assignments), 198.18.0.0/15 (RFC 2544 benchmarking, which several clouds
/// route to infrastructure), 240.0.0.0/4 (RFC 1112 reserved — also routed
/// internally in some clouds, and it subsumes the limited broadcast address),
/// the limited broadcast address, and all of 224.0.0.0/4 multicast.
///
/// The 0.0.0.0/8 rule is broader than `is_unspecified()` on purpose:
/// `is_unspecified()` matches only `0.0.0.0` exactly, so `https://0.1.2.3/`
/// passed this guard while many stacks still route it to the local host.
///
/// Loopback is deliberately *not* included here; callers decide whether to
/// exempt it, because it is the one range local tests legitimately need.
/// (Its IPv6 counterpart [`is_internal_v6`] *does* include loopback — see
/// there for why.)
///
/// Only Web Push validates arbitrary caller-supplied hosts; FCM and APNS just
/// need the loopback check above for their base-URL scheme rule.
#[cfg(feature = "web-push")]
pub(crate) fn is_internal_v4(ip: &Ipv4Addr) -> bool {
    let o = ip.octets();
    let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
    let protocol_assignments = o[0] == 192 && o[1] == 0 && o[2] == 0;
    // RFC 1122 "this network" — 0.0.0.0/8, not just the unspecified address.
    let this_network = o[0] == 0;
    // RFC 2544 benchmarking, 198.18.0.0/15 — routed to infrastructure in some
    // clouds rather than being genuinely dark.
    let benchmarking = o[0] == 198 && (o[1] == 18 || o[1] == 19);
    // RFC 1112 reserved, 240.0.0.0/4. Nominally unusable, but some clouds route
    // it internally, and nothing legitimate is a push endpoint there.
    let reserved = o[0] >= 240;

    ip.is_private()
        || ip.is_link_local()
        || this_network
        || ip.is_broadcast()
        || ip.is_multicast()
        || cgnat
        || protocol_assignments
        || benchmarking
        || reserved
}

/// True for IPv6 addresses that belong to internal ranges.
///
/// Every IPv6 form that can carry an embedded IPv4 address is resolved first
/// and delegated to the IPv4 rules, because none of the IPv6 prefix masks
/// match those forms and the `Host::Ipv4` arm is never consulted (the URL
/// parses as `Host::Ipv6`):
///
/// - **IPv4-mapped, `::ffff:a.b.c.d`** — the original bypass:
///   `https://[::ffff:169.254.169.254]/` reached the cloud metadata service.
/// - **IPv4-compatible, `::a.b.c.d`** — deprecated and usually unroutable,
///   but free to block, and covered for free by `to_ipv4()` (which matches
///   both forms, unlike `to_ipv4_mapped()`).
/// - **NAT64, `64:ff9b::/96`** — the important one. On an IPv6-only cluster
///   behind a NAT64/DNS64 gateway (the standard configuration for IPv6 EKS
///   and GKE), `https://[64:ff9b::a9fe:a9fe]/latest/meta-data/` is translated
///   by the gateway into a plain IPv4 request to 169.254.169.254. That is
///   precisely the bypass this guard exists to close.
/// - **6to4, `2002::/16`** — the embedded v4 sits in segments 1 and 2, so
///   `https://[2002:a9fe:a9fe::]/` reaches 169.254.169.254 on any host with a
///   6to4 tunnel.
/// - **IPv4-translated, `::ffff:0:a.b.c.d`** — the deprecated RFC 2765 form
///   (`s[4] == 0xffff && s[5] == 0`). `to_ipv4()` returns `None` for it, so it
///   matched no rule at all and fell through to "public".
/// - **Teredo, `2001::/32`** — the tunnel client's IPv4 address is stored as
///   the *complement* of the low 32 bits, so `2001:0:...:5601:5601` carries
///   169.254.169.254 and reads as an ordinary public v6 address without this.
///
/// Unlike [`is_internal_v4`], loopback *is* included here. The asymmetry was a
/// trap: plain `::1` returned `false`, and only `web_push.rs`'s separate
/// loopback branch kept it from being reachable. Any second caller of this
/// function would have inherited a loopback hole.
#[cfg(feature = "web-push")]
pub(crate) fn is_internal_v6(ip: &Ipv6Addr) -> bool {
    if ip.is_loopback() {
        return true;
    }

    // Covers both the IPv4-mapped (`::ffff:a.b.c.d`) and the deprecated
    // IPv4-compatible (`::a.b.c.d`) embeddings.
    if let Some(v4) = ip.to_ipv4() {
        return is_internal_v4(&v4) || v4.is_loopback();
    }

    let s = ip.segments();

    // IPv4-translated `::ffff:0:a.b.c.d` (RFC 2765). `to_ipv4()` above does not
    // match this form — it requires either an all-zero prefix or `s[5] ==
    // 0xffff` — so without this arm it matched no rule and passed as public.
    if s[0] == 0 && s[1] == 0 && s[2] == 0 && s[3] == 0 && s[4] == 0xffff && s[5] == 0 {
        let v4 = Ipv4Addr::from(((s[6] as u32) << 16) | s[7] as u32);
        return is_internal_v4(&v4) || v4.is_loopback();
    }

    // NAT64 well-known prefix 64:ff9b::/96 — the embedded v4 is the low 32 bits.
    if s[0] == 0x0064 && s[1] == 0xff9b {
        let v4 = Ipv4Addr::from(((s[6] as u32) << 16) | s[7] as u32);
        return is_internal_v4(&v4) || v4.is_loopback();
    }

    // 6to4 2002::/16 — the embedded v4 is segments 1 and 2.
    if s[0] == 0x2002 {
        let v4 = Ipv4Addr::from(((s[1] as u32) << 16) | s[2] as u32);
        return is_internal_v4(&v4) || v4.is_loopback();
    }

    // Teredo 2001::/32 — the tunnel client's IPv4 address is the *complement*
    // of the low 32 bits (RFC 4380 §4). Without un-complementing it, a Teredo
    // address wrapping 169.254.169.254 reads as an ordinary public v6 address.
    if s[0] == 0x2001 && s[1] == 0x0000 {
        let v4 = Ipv4Addr::from(!(((s[6] as u32) << 16) | s[7] as u32));
        return is_internal_v4(&v4) || v4.is_loopback();
    }

    // fe80::/10 link-local or fc00::/7 unique-local, plus the unspecified address.
    ip.is_unspecified() || (s[0] & 0xffc0) == 0xfe80 || (s[0] & 0xfe00) == 0xfc00
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

    #[test]
    fn nat64_wellknown_prefix_reaches_the_metadata_service() {
        // On an IPv6-only cluster behind a NAT64/DNS64 gateway this is
        // translated into a plain IPv4 request to 169.254.169.254.
        assert!(is_internal_v6(&v6("64:ff9b::a9fe:a9fe")));
        assert!(is_internal_v6(&v6("64:ff9b::169.254.169.254")));
        assert!(is_internal_v6(&v6("64:ff9b::10.0.0.1")));
        assert!(is_internal_v6(&v6("64:ff9b::127.0.0.1")));
        // A NAT64-embedded *public* address is still allowed through.
        assert!(!is_internal_v6(&v6("64:ff9b::93.184.216.34")));
    }

    #[test]
    fn six_to_four_embedded_v4_is_resolved() {
        // 2002:a9fe:a9fe:: carries 169.254.169.254 in segments 1 and 2.
        assert!(is_internal_v6(&v6("2002:a9fe:a9fe::")));
        assert!(is_internal_v6(&v6("2002:0a00:0001::")));
        assert!(is_internal_v6(&v6("2002:7f00:0001::")));
        // A 6to4 address wrapping a public v4 is not internal.
        assert!(!is_internal_v6(&v6("2002:5db8:d822::")));
    }

    #[test]
    fn ipv4_compatible_addresses_are_resolved() {
        // Deprecated `::a.b.c.d` form; `to_ipv4_mapped()` did not see it.
        assert!(is_internal_v6(&v6("::169.254.169.254")));
        assert!(is_internal_v6(&v6("::10.0.0.1")));
    }

    #[test]
    fn plain_ipv6_loopback_is_internal() {
        // Used to return false, leaving the loopback hole to `web_push.rs`'s
        // separate branch — a trap for the next caller of this function.
        assert!(is_internal_v6(&v6("::1")));
    }

    #[test]
    fn this_network_block_is_internal_not_just_the_unspecified_address() {
        // `is_unspecified()` matches only 0.0.0.0, so `https://0.1.2.3/` used
        // to pass the guard.
        assert!(is_internal_v4(&v4("0.0.0.0")));
        assert!(is_internal_v4(&v4("0.1.2.3")));
        assert!(is_internal_v4(&v4("0.255.255.255")));
        assert!(!is_internal_v4(&v4("1.0.0.1")));
    }

    #[test]
    fn ipv4_translated_form_is_internal() {
        // Closes: `::ffff:0:a.b.c.d` (RFC 2765 IPv4-translated). `to_ipv4()`
        // returns None for this spelling — it wants an all-zero prefix or
        // `s[5] == 0xffff` — so `https://[::ffff:0:169.254.169.254]/` matched
        // no rule and reached the cloud metadata service.
        assert!(is_internal_v6(&v6("::ffff:0:169.254.169.254")));
        assert!(is_internal_v6(&v6("::ffff:0:a9fe:a9fe")));
        assert!(is_internal_v6(&v6("::ffff:0:10.0.0.1")));
        assert!(is_internal_v6(&v6("::ffff:0:127.0.0.1")));
        // The prefix really is what distinguishes it: a translated *public*
        // address is still allowed through.
        assert!(!is_internal_v6(&v6("::ffff:0:93.184.216.34")));
    }

    #[test]
    fn teredo_embedded_client_v4_is_internal() {
        // Closes: Teredo 2001::/32 (RFC 4380 §4) stores the client IPv4 as the
        // complement of the low 32 bits, so an address wrapping
        // 169.254.169.254 (!0xa9fea9fe == 0x56015601) previously read as an
        // ordinary public IPv6 address.
        assert!(is_internal_v6(&v6("2001:0:4136:e378:8000:63bf:5601:5601")));
        // !10.0.0.1 == 0xf5fffffe
        assert!(is_internal_v6(&v6("2001:0::f5ff:fffe")));
        // !127.0.0.1 == 0x80fffffe
        assert!(is_internal_v6(&v6("2001:0::80ff:fffe")));
        // A Teredo address wrapping a public v4 stays allowed: !93.184.216.34
        // == 0xa24727dd.
        assert!(!is_internal_v6(&v6("2001:0:4136:e378:8000:63bf:a247:27dd")));
        // 2001:db8::/32 documentation space is *not* Teredo (s[1] != 0) and
        // must not be misread through the complement rule.
        assert!(!is_internal_v6(&v6("2001:db8::1")));
    }

    #[test]
    fn benchmarking_and_reserved_v4_ranges_are_internal() {
        // Closes: 198.18.0.0/15 (RFC 2544 benchmarking) and 240.0.0.0/4
        // (RFC 1112 reserved). Both are routed to infrastructure in some
        // clouds, and both previously passed as ordinary public addresses.
        assert!(is_internal_v4(&v4("198.18.0.1")));
        assert!(is_internal_v4(&v4("198.19.255.255")));
        assert!(is_internal_v4(&v4("240.0.0.1")));
        assert!(is_internal_v4(&v4("255.0.0.1")));
        // Boundaries: 198.17.x and 198.20.x are outside 198.18.0.0/15, and
        // 239.x is the top of the multicast block rather than the reserved one.
        assert!(!is_internal_v4(&v4("198.17.255.255")));
        assert!(!is_internal_v4(&v4("198.20.0.0")));
        assert!(!is_internal_v4(&v4("198.51.100.1")));
    }

    #[test]
    fn broadcast_and_multicast_are_internal() {
        assert!(is_internal_v4(&v4("255.255.255.255")));
        assert!(is_internal_v4(&v4("224.0.0.1")));
        assert!(is_internal_v4(&v4("239.255.255.250")));
        assert!(!is_internal_v4(&v4("223.255.255.255")));
    }
}
