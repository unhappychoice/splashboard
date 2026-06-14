//! `net_vpn` — host VPN connection state.
//!
//! An interface is treated as a VPN tunnel when its `netdev` type is `Tunnel` / `Ppp`, or its
//! name starts with one of the common VPN-driver prefixes (`tun` / `utun` / `wg` / `ipsec` / …).
//! Name-prefix detection is the load-bearing path on Linux because WireGuard and OpenVPN-tun
//! devices report as `Ether`, not `Tunnel`, in `/sys/class/net` and would otherwise slip past
//! the `if_type` check.

use async_trait::async_trait;

use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{NetIface, payload, snapshot};

const SHAPES: &[Shape] = &[
    Shape::Badge,
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
];

/// Interface-name prefixes that identify a VPN tunnel even when `netdev` doesn't classify the
/// `if_type` as `Tunnel`. Covers the userspace tun drivers (`tun*` / `utun*`) plus the kernel
/// modules from the major commercial / FOSS VPN stacks.
const VPN_NAME_PREFIXES: &[&str] = &[
    "tun", "utun", "wg", "ipsec", "nordlynx", "proton", "tap", "gpd", "cscotun",
];

pub struct NetVpn;

#[async_trait]
impl Fetcher for NetVpn {
    fn name(&self) -> &str {
        "net_vpn"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Whether this host is currently routing through a VPN tunnel. `Badge` (default) is a \
         connected / disconnected pill — `disconnected` is `Warn`, since the widget exists \
         because the user wants to know when the tunnel is *down*. `Text` names the active VPN \
         interface and its address; `TextBlock` / `MarkdownTextBlock` / `Entries` list every \
         active tunnel. Detects `netdev` `Tunnel` / `Ppp` types plus name-prefix fallback for \
         WireGuard, OpenVPN-tun, IPsec, NordLynx, ProtonVPN, Cisco AnyConnect, and GlobalProtect."
    }
    fn refresh_interval(&self) -> u64 {
        60
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_snapshot(), shape)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let ifaces = snapshot();
        let shape = ctx.shape.unwrap_or(Shape::Badge);
        Ok(payload(
            body_for_shape(&ifaces, shape).unwrap_or_else(|| Body::Badge(vpn_badge(&[]))),
        ))
    }
}

fn body_for_shape(ifaces: &[NetIface], shape: Shape) -> Option<Body> {
    let vpns = active_vpns(ifaces);
    Some(match shape {
        Shape::Badge => Body::Badge(vpn_badge(&vpns)),
        Shape::Text => Body::Text(TextData {
            value: headline(&vpns),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vpns.iter().map(|i| row_line(i)).collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: vpns
                .iter()
                .map(|i| format!("- **{}** {}", i.name, addr_summary(i)))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vpns
                .iter()
                .map(|i| Entry {
                    key: i.name.clone(),
                    value: Some(addr_summary(i)),
                    status: Some(Status::Ok),
                })
                .collect(),
        }),
        _ => return None,
    })
}

fn active_vpns(ifaces: &[NetIface]) -> Vec<&NetIface> {
    ifaces
        .iter()
        .filter(|i| is_vpn(i) && i.up && (!i.ipv4.is_empty() || !i.ipv6.is_empty()))
        .collect()
}

fn is_vpn(i: &NetIface) -> bool {
    if i.loopback {
        return false;
    }
    if i.kind == "Tunnel" || i.kind == "Ppp" {
        return true;
    }
    VPN_NAME_PREFIXES.iter().any(|p| i.name.starts_with(p))
}

fn headline(vpns: &[&NetIface]) -> String {
    match vpns.first() {
        None => "VPN: off".into(),
        Some(i) => format!("VPN: {} · {}", i.name, addr_summary(i)),
    }
}

fn row_line(i: &NetIface) -> String {
    format!("{}  {}", i.name, addr_summary(i))
}

fn addr_summary(i: &NetIface) -> String {
    [
        i.ipv4.first().map(ToString::to_string),
        i.ipv6.first().map(ToString::to_string),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" · ")
}

/// `disconnected` is `Warn`, not `Ok`: a user who configured this widget cares about the tunnel
/// being up, so the absence is the noteworthy state worth a glance flag (mirrors `net_proxy`'s
/// "proxied is the surprise" stance, with the direction flipped).
fn vpn_badge(vpns: &[&NetIface]) -> BadgeData {
    if vpns.is_empty() {
        BadgeData {
            status: Status::Warn,
            label: "disconnected".into(),
        }
    } else if vpns.len() == 1 {
        BadgeData {
            status: Status::Ok,
            label: format!("connected · {}", vpns[0].name),
        }
    } else {
        BadgeData {
            status: Status::Ok,
            label: format!("connected · {} tunnels", vpns.len()),
        }
    }
}

fn sample_snapshot() -> Vec<NetIface> {
    vec![
        NetIface {
            name: "lo".into(),
            kind: "Loopback".into(),
            mac: None,
            ipv4: vec![std::net::Ipv4Addr::LOCALHOST],
            ipv6: vec![],
            up: true,
            loopback: true,
            is_default: false,
            mtu: Some(65536),
            rx_bytes: 0,
            tx_bytes: 0,
        },
        NetIface {
            name: "eth0".into(),
            kind: "Ethernet".into(),
            mac: Some("aa:bb:cc:dd:ee:ff".into()),
            ipv4: vec![std::net::Ipv4Addr::new(192, 168, 1, 24)],
            ipv6: vec![],
            up: true,
            loopback: false,
            is_default: true,
            mtu: Some(1500),
            rx_bytes: 0,
            tx_bytes: 0,
        },
        NetIface {
            name: "tun0".into(),
            kind: "Tunnel".into(),
            mac: None,
            ipv4: vec![std::net::Ipv4Addr::new(10, 8, 0, 5)],
            ipv6: vec![],
            up: true,
            loopback: false,
            is_default: false,
            mtu: Some(1380),
            rx_bytes: 0,
            tx_bytes: 0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::tests::iface;
    use super::*;

    fn vpn_iface(name: &str, kind: &str, ipv4: &[&str]) -> NetIface {
        let mut i = iface(name, ipv4, true);
        i.kind = kind.into();
        i.loopback = false;
        i
    }

    #[test]
    fn body_for_shape_covers_every_supported_shape() {
        let snap = sample_snapshot();
        for &shape in SHAPES {
            let body = body_for_shape(&snap, shape).unwrap();
            assert_eq!(crate::render::shape_of(&body), shape);
        }
        assert!(body_for_shape(&snap, Shape::Ratio).is_none());
        assert!(body_for_shape(&snap, Shape::Bars).is_none());
        assert!(body_for_shape(&snap, Shape::Timeline).is_none());
    }

    #[test]
    fn is_vpn_recognises_kind_and_name_prefixes() {
        assert!(is_vpn(&vpn_iface("tun0", "Ethernet", &["10.0.0.1"])));
        assert!(is_vpn(&vpn_iface("utun3", "Ethernet", &["10.0.0.1"])));
        assert!(is_vpn(&vpn_iface("wg0", "Ethernet", &["10.0.0.1"])));
        assert!(is_vpn(&vpn_iface("ipsec0", "Ethernet", &["10.0.0.1"])));
        assert!(is_vpn(&vpn_iface("nordlynx", "Ethernet", &["10.0.0.1"])));
        assert!(is_vpn(&vpn_iface("anything", "Tunnel", &["10.0.0.1"])));
        assert!(is_vpn(&vpn_iface("anything", "Ppp", &["10.0.0.1"])));

        assert!(!is_vpn(&vpn_iface("eth0", "Ethernet", &["10.0.0.1"])));
        assert!(!is_vpn(&vpn_iface(
            "wlan0",
            "Wireless IEEE 802.11",
            &["10.0.0.1"]
        )));
        let lo = iface("lo", &["127.0.0.1"], true);
        assert!(!is_vpn(&lo));
    }

    #[test]
    fn active_vpns_requires_up_and_addressed() {
        let down_vpn = {
            let mut i = vpn_iface("tun0", "Tunnel", &["10.8.0.5"]);
            i.up = false;
            i
        };
        let no_addr_vpn = vpn_iface("wg0", "Ethernet", &[]);
        let live = vpn_iface("utun1", "Ethernet", &["10.99.0.2"]);
        let snap = vec![down_vpn, no_addr_vpn, live];
        let vpns = active_vpns(&snap);
        assert_eq!(vpns.len(), 1);
        assert_eq!(vpns[0].name, "utun1");
    }

    #[test]
    fn badge_distinguishes_disconnected_single_and_multi() {
        let none: Vec<&NetIface> = vec![];
        let b0 = vpn_badge(&none);
        assert_eq!(b0.status, Status::Warn);
        assert_eq!(b0.label, "disconnected");

        let one = vpn_iface("tun0", "Tunnel", &["10.0.0.1"]);
        let b1 = vpn_badge(&[&one]);
        assert_eq!(b1.status, Status::Ok);
        assert!(b1.label.contains("tun0"));

        let a = vpn_iface("tun0", "Tunnel", &["10.0.0.1"]);
        let b = vpn_iface("wg0", "Ethernet", &["10.99.0.2"]);
        let b2 = vpn_badge(&[&a, &b]);
        assert!(b2.label.contains("2 tunnels"));
    }

    #[test]
    fn headline_names_first_active_tunnel_then_falls_back_to_off() {
        let snap = sample_snapshot();
        let vpns = active_vpns(&snap);
        let h = headline(&vpns);
        assert!(h.starts_with("VPN: tun0"));
        assert!(h.contains("10.8.0.5"));

        let nothing = vec![iface("eth0", &["10.0.0.1"], true)];
        let vpns_none = active_vpns(&nothing);
        assert_eq!(headline(&vpns_none), "VPN: off");
    }

    #[test]
    fn entries_carry_status_ok_for_every_active_tunnel() {
        let snap = vec![
            vpn_iface("tun0", "Tunnel", &["10.8.0.5"]),
            vpn_iface("wg0", "Ethernet", &["10.99.0.2"]),
        ];
        let Body::Entries(e) = body_for_shape(&snap, Shape::Entries).unwrap() else {
            panic!("expected entries");
        };
        assert_eq!(e.items.len(), 2);
        assert!(e.items.iter().all(|i| i.status == Some(Status::Ok)));
    }

    #[test]
    fn addr_summary_joins_v4_and_v6_when_both_present() {
        let mut i = vpn_iface("tun0", "Tunnel", &["10.0.0.1"]);
        i.ipv6 = vec!["fd00::1".parse().unwrap()];
        let s = addr_summary(&i);
        assert!(s.contains("10.0.0.1"));
        assert!(s.contains("fd00::1"));
    }

    #[tokio::test]
    async fn fetch_emits_the_requested_shape() {
        for &shape in SHAPES {
            let ctx = FetchContext {
                shape: Some(shape),
                ..Default::default()
            };
            let body = NetVpn.fetch(&ctx).await.unwrap().body;
            // On a CI host with no VPN, every shape still resolves — just empty multi-row shapes.
            assert_eq!(crate::render::shape_of(&body), shape);
        }
    }

    #[test]
    fn fetcher_contract_metadata() {
        assert_eq!(NetVpn.name(), "net_vpn");
        assert_eq!(NetVpn.safety(), Safety::Safe);
        assert_eq!(NetVpn.default_shape(), Shape::Badge);
        assert!(NetVpn.option_schemas().is_empty());
        let description = NetVpn.description();
        assert!(description.contains("Badge"), "{description}");
        assert!(description.contains("VPN"), "{description}");
        for &shape in SHAPES {
            assert!(NetVpn.sample_body(shape).is_some());
        }
        assert!(NetVpn.sample_body(Shape::Ratio).is_none());
    }
}
