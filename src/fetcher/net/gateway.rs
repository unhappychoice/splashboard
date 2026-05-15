//! `net_gateway` — the host's default-route gateway.

use async_trait::async_trait;
use serde::Deserialize;

use crate::options::OptionSchema;
use crate::payload::{
    BadgeData, Body, EntriesData, MarkdownTextBlockData, Payload, Status, TextBlockData, TextData,
};
use crate::render::Shape;

use super::super::{FetchContext, FetchError, Fetcher, Safety};
use super::{
    GatewayInfo, cache_key, default_gateway, entry, options_placeholder, parse_options, payload,
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
    type_hint: "\"gateway\" | \"interface\"",
    required: false,
    default: Some("\"gateway\""),
    description: "Which field the `Text` shape shows — the gateway address or the interface it routes through. Ignored by the other shapes. (Route metric isn't exposed by the underlying platform APIs, so it isn't a `kind`.)",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayOptions {
    #[serde(default)]
    pub kind: Option<GatewayKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayKind {
    #[default]
    Gateway,
    Interface,
}

pub struct NetGateway;

#[async_trait]
impl Fetcher for NetGateway {
    fn name(&self) -> &str {
        "net_gateway"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "The host's default-route gateway. `Text` (default) shows the gateway address — `kind = \"interface\"` shows the interface it routes through instead; `TextBlock` / `MarkdownTextBlock` / `Entries` roll up gateway + interface; `Badge` is a has-default-route / no-route pill."
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
        body_for_shape(&sample_gateway(), shape, GatewayKind::Gateway)
    }
    async fn fetch(&self, ctx: &FetchContext) -> Result<Payload, FetchError> {
        let opts: GatewayOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return Ok(options_placeholder(&msg)),
        };
        let gw = default_gateway();
        let shape = ctx.shape.unwrap_or(Shape::Text);
        let kind = opts.kind.unwrap_or_default();
        Ok(payload(body_for_shape(&gw, shape, kind).unwrap_or_else(
            || {
                Body::Text(TextData {
                    value: text_value(&gw, kind),
                })
            },
        )))
    }
}

fn body_for_shape(gw: &GatewayInfo, shape: Shape, kind: GatewayKind) -> Option<Body> {
    Some(match shape {
        Shape::Text => Body::Text(TextData {
            value: text_value(gw, kind),
        }),
        Shape::TextBlock => Body::TextBlock(TextBlockData {
            lines: vec![
                format!("gateway  {}", gateway_field(gw)),
                format!("interface  {}", interface_field(gw)),
            ],
        }),
        Shape::MarkdownTextBlock => Body::MarkdownTextBlock(MarkdownTextBlockData {
            value: format!(
                "- **gateway** {}\n- **interface** {}",
                gateway_field(gw),
                interface_field(gw),
            ),
        }),
        Shape::Entries => Body::Entries(EntriesData {
            items: vec![
                entry("gateway", &gateway_field(gw)),
                entry("interface", &interface_field(gw)),
            ],
        }),
        Shape::Badge => Body::Badge(gateway_badge(gw)),
        _ => return None,
    })
}

fn text_value(gw: &GatewayInfo, kind: GatewayKind) -> String {
    match kind {
        GatewayKind::Gateway => gw
            .ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "no default route".into()),
        GatewayKind::Interface => interface_field(gw),
    }
}

fn gateway_field(gw: &GatewayInfo) -> String {
    gw.ip
        .map(|ip| ip.to_string())
        .unwrap_or_else(|| "n/a".into())
}

fn interface_field(gw: &GatewayInfo) -> String {
    gw.interface.clone().unwrap_or_else(|| "n/a".into())
}

fn gateway_badge(gw: &GatewayInfo) -> BadgeData {
    match gw.ip {
        Some(ip) => BadgeData {
            status: Status::Ok,
            label: ip.to_string(),
        },
        None => BadgeData {
            status: Status::Error,
            label: "no default route".into(),
        },
    }
}

fn sample_gateway() -> GatewayInfo {
    GatewayInfo {
        ip: Some(std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            192, 168, 1, 1,
        ))),
        interface: Some("eth0".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_for_shape_covers_every_supported_shape() {
        let gw = sample_gateway();
        for &shape in SHAPES {
            let body = body_for_shape(&gw, shape, GatewayKind::Gateway).unwrap();
            assert_eq!(crate::render::shape_of(&body), shape);
        }
        assert!(body_for_shape(&gw, Shape::Ratio, GatewayKind::Gateway).is_none());
    }

    #[test]
    fn text_value_honours_kind() {
        let gw = sample_gateway();
        assert_eq!(text_value(&gw, GatewayKind::Gateway), "192.168.1.1");
        assert_eq!(text_value(&gw, GatewayKind::Interface), "eth0");

        let empty = GatewayInfo::default();
        assert_eq!(text_value(&empty, GatewayKind::Gateway), "no default route");
        assert_eq!(text_value(&empty, GatewayKind::Interface), "n/a");
    }

    #[test]
    fn badge_reflects_route_presence() {
        assert_eq!(gateway_badge(&sample_gateway()).status, Status::Ok);
        assert_eq!(gateway_badge(&GatewayInfo::default()).status, Status::Error);
    }

    #[test]
    fn entries_always_carry_both_rows_even_when_unresolved() {
        let Body::Entries(d) = body_for_shape(
            &GatewayInfo::default(),
            Shape::Entries,
            GatewayKind::Gateway,
        )
        .unwrap() else {
            panic!("expected entries");
        };
        assert_eq!(d.items.len(), 2);
        assert_eq!(d.items[0].value.as_deref(), Some("n/a"));
    }

    #[test]
    fn options_parse_and_reject_unknown_keys() {
        let ok: toml::Value = toml::from_str("kind = \"interface\"").unwrap();
        let parsed: GatewayOptions = parse_options(Some(&ok)).unwrap();
        assert!(matches!(parsed.kind, Some(GatewayKind::Interface)));
        let bad: toml::Value = toml::from_str("metric = 1").unwrap();
        assert!(parse_options::<GatewayOptions>(Some(&bad)).is_err());
    }

    #[tokio::test]
    async fn fetch_rejects_invalid_options_to_placeholder() {
        let ctx = FetchContext {
            shape: Some(Shape::Text),
            options: Some(toml::from_str("kind = \"metric\"").unwrap()),
            ..Default::default()
        };
        let Body::Text(t) = NetGateway.fetch(&ctx).await.unwrap().body else {
            panic!("expected text");
        };
        assert!(t.value.starts_with("⚠"));
    }

    #[test]
    fn fetcher_contract_metadata() {
        assert_eq!(NetGateway.name(), "net_gateway");
        assert_eq!(NetGateway.safety(), Safety::Safe);
        assert_eq!(NetGateway.default_shape(), Shape::Text);
        assert_eq!(NetGateway.option_schemas().len(), 1);
        for &shape in SHAPES {
            assert!(NetGateway.sample_body(shape).is_some());
        }
        assert!(NetGateway.sample_body(Shape::Bars).is_none());
    }

    #[test]
    fn description_and_refresh_interval_are_populated() {
        // `description()` and `refresh_interval()` round out the catalog metadata surface but
        // weren't hit by the bare contract checks above — keep them honest so a copy-paste of the
        // fetcher template can't silently leave either empty / unset.
        assert!(NetGateway.description().contains("default-route"));
        assert_eq!(NetGateway.refresh_interval(), 60 * 10);
    }

    #[test]
    fn cache_key_is_name_prefixed_and_options_sensitive() {
        // The fetcher delegates to the shared `super::cache_key` helper; pin the name prefix plus
        // options-partitioning so a regression in the family helper can't drift the gateway slot
        // into a sibling fetcher's cache.
        let base = FetchContext {
            shape: Some(Shape::Text),
            ..Default::default()
        };
        let with_opts = FetchContext {
            options: Some(toml::from_str("kind = \"interface\"").unwrap()),
            ..base.clone()
        };
        assert!(NetGateway.cache_key(&base).starts_with("net_gateway-"));
        assert_ne!(
            NetGateway.cache_key(&base),
            NetGateway.cache_key(&with_opts)
        );
    }

    #[tokio::test]
    async fn fetch_unsupported_shape_falls_back_to_text() {
        // `body_for_shape` returns `None` for any shape outside `SHAPES`; `fetch` then materialises
        // a `Text` fallback so a misconfigured widget still renders rather than dropping the body.
        let ctx = FetchContext {
            shape: Some(Shape::Ratio),
            ..Default::default()
        };
        let body = NetGateway.fetch(&ctx).await.unwrap().body;
        assert!(matches!(&body, Body::Text(t) if !t.value.is_empty()));
    }
}
