//! `net_ip` — the host's local IP addresses.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, Entry, MarkdownTextBlockData, Payload, Status, TextBlockData,
    TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{
    NetIface, cache_key, options_placeholder, parse_options, payload, primary_interface, snapshot,
};

const SHAPES: &[Shape] = &[
    Shape::Text,
    Shape::TextBlock,
    Shape::MarkdownTextBlock,
    Shape::Entries,
    Shape::Badge,
];

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"primary\" | \"v4\" | \"v6\"",
    required: false,
    default: Some("\"primary\""),
    description: "Which address the `Text` shape shows for the primary interface. Ignored by `TextBlock` / `MarkdownTextBlock` / `Entries` (which always list every interface) and `Badge`.",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IpOptions {
    #[serde(default)]
    pub kind: Option<IpKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IpKind {
    #[default]
    Primary,
    V4,
    V6,
}

pub struct NetIp;

#[async_trait]
impl Fetcher for NetIp {
    fn name(&self) -> &str {
        "net_ip"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The host's own IPv4 / IPv6 addresses. `Text` (default) shows one address for the primary (default-route) interface — `kind` picks v4 / v6 / first-available; `TextBlock` / `MarkdownTextBlock` / `Entries` list every interface that has an address; `Badge` flags dual-stack / IPv4 / IPv6-only / no-address."
    }
    fn refresh_interval(&self) -> u64 {
        60 * 10
    }
    fn shapes(&self) -> &[Shape] {
        SHAPES
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn cache_key(&self, ctx: &FetchContext) -> String {
        cache_key(self.name(), ctx)
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        body_for_shape(&sample_snapshot(), shape, IpKind::Primary)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: IpOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let ifaces = snapshot();
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let kind = opts.kind.unwrap_or_default();
        Ok(payload(
            body_for_shape(&ifaces, shape, kind).unwrap_or_else(|| {
                Body::Text(TextData {
                    value: text_value(&ifaces, kind),
                })
            }),
        ))
    }
}

fn body_for_shape(ifaces: &[NetIface], shape: Shape, kind: IpKind) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: text_value(ifaces, kind),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: addressed(ifaces)
                .map(|i| format!("{}  {}", i.name, addr_strings(i).join(", ")))
                .collect(),
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: addressed(ifaces)
                .map(|i| format!("- **{}** {}", i.name, addr_strings(i).join(" · ")))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: addressed(ifaces)
                .map(|i| Entry {
                    key: i.name.clone(),
                    value: Some(addr_strings(i).join(", ")),
                    status: None,
                })
                .collect(),
        }),
        Shape::Badge => Body::Badge(ip_badge(ifaces)),
        _ => return None,
    })
}

fn text_value(ifaces: &[NetIface], kind: IpKind) -> String {
    let Some(i) = primary_interface(ifaces) else {
        return "no address".into();
    };
    match kind {
        IpKind::V4 => first_v4(i).unwrap_or_else(|| "no IPv4".into()),
        IpKind::V6 => first_v6(i).unwrap_or_else(|| "no IPv6".into()),
        IpKind::Primary => first_v4(i)
            .or_else(|| first_v6(i))
            .unwrap_or_else(|| "no address".into()),
    }
}

fn first_v4(i: &NetIface) -> Option<String> {
    i.ipv4.first().map(ToString::to_string)
}

fn first_v6(i: &NetIface) -> Option<String> {
    i.ipv6.first().map(ToString::to_string)
}

fn addr_strings(i: &NetIface) -> Vec<String> {
    i.ipv4
        .iter()
        .map(ToString::to_string)
        .chain(i.ipv6.iter().map(ToString::to_string))
        .collect()
}

fn addressed(ifaces: &[NetIface]) -> impl Iterator<Item = &NetIface> {
    ifaces
        .iter()
        .filter(|i| !i.ipv4.is_empty() || !i.ipv6.is_empty())
}

fn ip_badge(ifaces: &[NetIface]) -> BadgeData {
    match primary_interface(ifaces) {
        Some(i) if !i.ipv4.is_empty() && !i.ipv6.is_empty() => BadgeData {
            status: Status::Ok,
            label: "dual-stack".into(),
        },
        Some(i) if !i.ipv4.is_empty() => BadgeData {
            status: Status::Ok,
            label: format!("IPv4 {}", i.ipv4[0]),
        },
        Some(i) if !i.ipv6.is_empty() => BadgeData {
            status: Status::Ok,
            label: "IPv6 only".into(),
        },
        _ => BadgeData {
            status: Status::Error,
            label: "no address".into(),
        },
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
            ipv6: vec!["fe80::1ab2".parse().unwrap()],
            up: true,
            loopback: false,
            is_default: true,
            mtu: Some(1500),
            rx_bytes: 0,
            tx_bytes: 0,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::tests::iface;
    use super::*;

    #[test]
    fn body_for_shape_covers_every_supported_shape() {
        let snap = sample_snapshot();
        for &shape in SHAPES {
            let body = body_for_shape(&snap, shape, IpKind::Primary).unwrap();
            assert_eq!(crate::render::shape_of(&body), shape);
        }
        assert!(body_for_shape(&snap, Shape::Ratio, IpKind::Primary).is_none());
        assert!(body_for_shape(&snap, Shape::Bars, IpKind::Primary).is_none());
    }

    #[test]
    fn text_value_honours_kind_and_falls_back() {
        let snap = sample_snapshot();
        assert_eq!(text_value(&snap, IpKind::Primary), "192.168.1.24");
        assert_eq!(text_value(&snap, IpKind::V4), "192.168.1.24");
        assert_eq!(text_value(&snap, IpKind::V6), "fe80::1ab2");

        let v6_only = vec![{
            let mut i = iface("eth0", &[], true);
            i.is_default = true;
            i.ipv6 = vec!["fe80::9".parse().unwrap()];
            i
        }];
        assert_eq!(text_value(&v6_only, IpKind::Primary), "fe80::9");
        assert_eq!(text_value(&v6_only, IpKind::V4), "no IPv4");
        assert_eq!(text_value(&[], IpKind::Primary), "no address");
    }

    /// `IpKind::V6` on a v4-only default interface hits the `first_v6(i).unwrap_or_else`
    /// fall-back arm — symmetrical to the v4 case above, but unreachable from the dual-stack
    /// `sample_snapshot()` where the default eth0 already carries an IPv6 address.
    #[test]
    fn text_value_v6_kind_falls_back_when_default_interface_has_no_ipv6() {
        let v4_only = vec![{
            let mut i = iface("eth0", &["10.0.0.2"], true);
            i.is_default = true;
            i
        }];
        assert_eq!(text_value(&v4_only, IpKind::V6), "no IPv6");
    }

    /// `IpKind::Primary` short-circuits to "no address" when the default interface exists but
    /// carries neither IPv4 nor IPv6. The `text_value(&[], IpKind::Primary)` case above only
    /// covers the `primary_interface(...) == None` arm at the top of the function; this drives
    /// the `first_v4(i).or_else(first_v6(i)).unwrap_or_else(...)` chain to its terminal fallback.
    #[test]
    fn text_value_primary_returns_no_address_when_default_interface_has_no_addresses() {
        let no_addrs = vec![{
            let mut i = iface("eth0", &[], true);
            i.is_default = true;
            i
        }];
        assert_eq!(text_value(&no_addrs, IpKind::Primary), "no address");
    }

    #[test]
    fn entries_skip_interfaces_without_an_address() {
        let snap = vec![
            iface("eth0", &["10.0.0.2"], true),
            iface("wlan0", &[], true),
        ];
        let Body::Entries(d) = body_for_shape(&snap, Shape::Entries, IpKind::Primary).unwrap()
        else {
            panic!("expected entries");
        };
        assert_eq!(d.items.len(), 1);
        assert_eq!(d.items[0].key, "eth0");
    }

    #[test]
    fn badge_classifies_stack() {
        let dual = sample_snapshot();
        assert_eq!(ip_badge(&dual).label, "dual-stack");

        let mut v4 = iface("eth0", &["10.0.0.2"], true);
        v4.is_default = true;
        assert!(ip_badge(&[v4]).label.starts_with("IPv4"));
        assert_eq!(ip_badge(&[]).status, Status::Error);
    }

    /// IPv6-only default interface → the `Some(i) if !i.ipv6.is_empty()` arm of `ip_badge`,
    /// distinct from the IPv4-only and dual-stack arms exercised in `badge_classifies_stack`.
    /// The label is a fixed string (no address content) so the rendered badge stays
    /// stable-width when the host roams across networks.
    #[test]
    fn badge_reports_ipv6_only_when_default_interface_has_no_ipv4() {
        let mut v6_only = iface("eth0", &[], true);
        v6_only.is_default = true;
        v6_only.ipv6 = vec!["fe80::1ab2".parse().unwrap()];
        let badge = ip_badge(&[v6_only]);
        assert_eq!(badge.label, "IPv6 only");
        assert_eq!(badge.status, Status::Ok);
    }

    #[test]
    fn options_parse_and_reject_unknown_keys() {
        let ok: toml::Value = toml::from_str("kind = \"v6\"").unwrap();
        let parsed: IpOptions = parse_options(Some(&ok)).unwrap();
        assert!(matches!(parsed.kind, Some(IpKind::V6)));
        let bad: toml::Value = toml::from_str("bogus = 1").unwrap();
        assert!(parse_options::<IpOptions>(Some(&bad)).is_err());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options_to_placeholder() {
        let ctx = FetchContext {
            shape: Some(Shape::Text),
            options: Some(toml::from_str("kind = \"bogus\"").unwrap()),
            ..Default::default()
        };
        let Body::Text(t) = NetIp.fetch(&ctx).await.unwrap().body else {
            panic!("expected text");
        };
        assert!(t.value.starts_with("⚠"));
    }

    #[test]
    fn fetcher_contract_metadata() {
        assert_eq!(NetIp.name(), "net_ip");
        assert_eq!(NetIp.safety(), Safety::Safe);
        assert_eq!(NetIp.default_shape(), Shape::Text);
        assert_eq!(NetIp.option_schemas().len(), 1);
        let description = NetIp.description();
        // Each declared shape's role is named in the catalog blurb so the description doubles
        // as in-CLI documentation; check the family of mentions rather than the exact string.
        assert!(description.contains("Text"), "{description}");
        assert!(description.contains("Badge"), "{description}");
        for &shape in SHAPES {
            assert!(NetIp.sample_body(shape).is_some());
        }
        assert!(NetIp.sample_body(Shape::Ratio).is_none());
        let base = FetchContext {
            shape: Some(Shape::Text),
            ..Default::default()
        };
        let other = FetchContext {
            options: Some(toml::from_str("kind = \"v6\"").unwrap()),
            ..base.clone()
        };
        assert_ne!(NetIp.cache_key(&base), NetIp.cache_key(&other));
    }
}
