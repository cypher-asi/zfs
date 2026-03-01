use libp2p::multiaddr::Protocol;
use libp2p::Multiaddr;

/// Strip all `/p2p/<peer_id>` components from a multiaddr, leaving only the
/// transport portion. Safe to call on addresses that have no `/p2p/` component.
pub fn strip_all_p2p(addr: &Multiaddr) -> Multiaddr {
    addr.iter()
        .filter(|p| !matches!(p, Protocol::P2p(_)))
        .collect()
}

/// Normalize a peer multiaddr for use in Kademlia.
///
/// Direct addresses are reduced to their transport portion (all `/p2p/`
/// components stripped) because the peer ID is stored as the Kademlia
/// record key.
///
/// Circuit addresses are rebuilt into canonical form:
/// `<transport>/p2p/<relay>/p2p-circuit/p2p/<dest>`, removing any duplicate
/// segments.
pub fn normalize_multiaddr(addr: &Multiaddr) -> Multiaddr {
    let has_circuit = addr.iter().any(|p| matches!(p, Protocol::P2pCircuit));

    if !has_circuit {
        return strip_all_p2p(addr);
    }

    let mut transport = Multiaddr::empty();
    let mut relay_peer = None;
    let mut dest_peer = None;
    let mut past_circuit = false;

    for proto in addr.iter() {
        if matches!(&proto, Protocol::P2pCircuit) {
            past_circuit = true;
            continue;
        }
        if let Protocol::P2p(ref peer) = proto {
            if past_circuit {
                if dest_peer.is_none() {
                    dest_peer = Some(*peer);
                }
            } else {
                relay_peer = Some(*peer);
            }
            continue;
        }
        if !past_circuit {
            transport = transport.with(proto);
        }
    }

    let mut result = transport;
    if let Some(relay) = relay_peer {
        result = result.with(Protocol::P2p(relay));
    }
    result = result.with(Protocol::P2pCircuit);
    if let Some(dest) = dest_peer {
        result = result.with(Protocol::P2p(dest));
    }
    result
}

/// Normalize a multiaddr while preserving exactly one trailing `/p2p/<peer>`
/// component for direct addresses. Use this when the resulting address must
/// remain dial-ready (e.g. for persistence or bootstrap lists).
///
/// Circuit addresses are normalized identically to [`normalize_multiaddr`].
pub fn sanitize_dial_addr(addr: &Multiaddr) -> Multiaddr {
    let has_circuit = addr.iter().any(|p| matches!(p, Protocol::P2pCircuit));

    if has_circuit {
        return normalize_multiaddr(addr);
    }

    let mut last_peer = None;
    for p in addr.iter() {
        if let Protocol::P2p(peer) = p {
            last_peer = Some(peer);
        }
    }

    let transport = strip_all_p2p(addr);
    match last_peer {
        Some(peer) => transport.with(Protocol::P2p(peer)),
        None => transport,
    }
}

/// Extract the first `PeerId` from a multiaddr's `/p2p/<peer>` component.
pub fn extract_peer_id(addr: &Multiaddr) -> Option<libp2p::PeerId> {
    addr.iter().find_map(|proto| match proto {
        Protocol::P2p(peer_id) => Some(peer_id),
        _ => None,
    })
}

/// Returns `true` when the multiaddr contains at least one transport component
/// (IP + port), as opposed to a bare `/p2p/<peer_id>` with no dialable address.
pub fn has_transport(addr: &Multiaddr) -> bool {
    addr.iter().any(|p| {
        matches!(
            p,
            Protocol::Ip4(_) | Protocol::Ip6(_) | Protocol::Dns(_) | Protocol::Dns4(_) | Protocol::Dns6(_)
        )
    })
}

/// Extract the first IP address (v4 or v6) from a multiaddr as a string.
pub fn extract_ip(addr: &Multiaddr) -> Option<String> {
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => return Some(ip.to_string()),
            Protocol::Ip6(ip) => return Some(ip.to_string()),
            _ => {}
        }
    }
    None
}

/// Returns `true` when every IP address in the multiaddr is globally routable
/// (not loopback, private, link-local, or unspecified).
///
/// Addresses containing no IP component (e.g. DNS names) are considered
/// routable.
pub fn is_globally_routable(addr: &Multiaddr) -> bool {
    for proto in addr.iter() {
        match proto {
            Protocol::Ip4(ip) => {
                if ip.is_loopback()
                    || ip.is_private()
                    || ip.is_link_local()
                    || ip.is_unspecified()
                    || ip.is_broadcast()
                {
                    return false;
                }
            }
            Protocol::Ip6(ip) => {
                if ip.is_loopback() || ip.is_unspecified() {
                    return false;
                }
            }
            _ => {}
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use libp2p::PeerId;

    #[test]
    fn strip_all_p2p_removes_peer_ids() {
        let peer = PeerId::random();
        let addr: Multiaddr = format!("/ip4/1.2.3.4/tcp/3691/p2p/{peer}").parse().unwrap();
        let stripped = strip_all_p2p(&addr);
        assert_eq!(
            stripped,
            "/ip4/1.2.3.4/tcp/3691".parse::<Multiaddr>().unwrap()
        );
    }

    #[test]
    fn normalize_removes_duplicate_p2p_from_direct() {
        let peer = PeerId::random();
        let addr: Multiaddr = format!("/ip4/1.2.3.4/tcp/3691/p2p/{peer}/p2p/{peer}/p2p/{peer}")
            .parse()
            .unwrap();
        let normalized = normalize_multiaddr(&addr);
        assert_eq!(
            normalized,
            "/ip4/1.2.3.4/tcp/3691".parse::<Multiaddr>().unwrap()
        );
    }

    #[test]
    fn normalize_preserves_canonical_circuit() {
        let relay = PeerId::random();
        let dest = PeerId::random();
        let raw = format!("/ip4/1.2.3.4/tcp/3691/p2p/{relay}/p2p-circuit/p2p/{dest}");
        let addr: Multiaddr = raw.parse().unwrap();
        let normalized = normalize_multiaddr(&addr);
        assert_eq!(normalized, addr);
    }

    #[test]
    fn normalize_circuit_strips_extra_p2p() {
        let relay = PeerId::random();
        let dest = PeerId::random();
        let addr: Multiaddr = format!(
            "/ip4/1.2.3.4/tcp/3691/p2p/{relay}/p2p/{relay}/p2p-circuit/p2p/{dest}/p2p/{dest}"
        )
        .parse()
        .unwrap();
        let expected: Multiaddr =
            format!("/ip4/1.2.3.4/tcp/3691/p2p/{relay}/p2p-circuit/p2p/{dest}")
                .parse()
                .unwrap();
        assert_eq!(normalize_multiaddr(&addr), expected);
    }

    #[test]
    fn sanitize_dial_keeps_one_p2p() {
        let peer = PeerId::random();
        let addr: Multiaddr = format!("/ip4/1.2.3.4/tcp/3691/p2p/{peer}/p2p/{peer}")
            .parse()
            .unwrap();
        let expected: Multiaddr = format!("/ip4/1.2.3.4/tcp/3691/p2p/{peer}").parse().unwrap();
        assert_eq!(sanitize_dial_addr(&addr), expected);
    }

    #[test]
    fn sanitize_dial_preserves_clean_direct() {
        let peer = PeerId::random();
        let raw = format!("/ip4/1.2.3.4/tcp/3691/p2p/{peer}");
        let addr: Multiaddr = raw.parse().unwrap();
        assert_eq!(sanitize_dial_addr(&addr), addr);
    }

    #[test]
    fn globally_routable_public_ip() {
        let addr: Multiaddr = "/ip4/3.129.15.45/tcp/3691".parse().unwrap();
        assert!(is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_loopback() {
        let addr: Multiaddr = "/ip4/127.0.0.1/tcp/3691".parse().unwrap();
        assert!(!is_globally_routable(&addr));
    }

    #[test]
    fn not_routable_private_ranges() {
        for s in [
            "/ip4/172.31.43.222/tcp/3691",
            "/ip4/10.0.0.1/tcp/3691",
            "/ip4/192.168.1.1/tcp/3691",
        ] {
            let addr: Multiaddr = s.parse().unwrap();
            assert!(!is_globally_routable(&addr), "{s} should not be routable");
        }
    }

    #[test]
    fn extract_ip_v4() {
        let addr: Multiaddr = "/ip4/3.129.15.45/tcp/3691".parse().unwrap();
        assert_eq!(extract_ip(&addr), Some("3.129.15.45".to_string()));
    }

    #[test]
    fn extract_ip_none_for_bare_p2p() {
        let peer = PeerId::random();
        let addr: Multiaddr = format!("/p2p/{peer}").parse().unwrap();
        assert_eq!(extract_ip(&addr), None);
    }

    #[test]
    fn no_ip_considered_routable() {
        let peer = PeerId::random();
        let addr: Multiaddr = format!("/p2p/{peer}").parse().unwrap();
        assert!(is_globally_routable(&addr));
    }
}
