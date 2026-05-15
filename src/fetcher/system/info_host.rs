//! `system_info_host` — single static host identifier picked by `kind`.
//!
//! Covers terminal / OS / hostname / shell / arch. Text-only. The dynamic rollup
//! (uptime / load / cpu% / mem%) lives on `system_monitor_host`.

use serde::Deserialize;
use sysinfo::System;

use crate::options::OptionSchema;
use crate::payload::{Body, Payload, TextData};
use crate::render::Shape;
use crate::samples;

use super::super::{FetchContext, RealtimeFetcher, Safety};
use super::{detect_shell, detect_terminal, options_placeholder, os_label, parse_options, payload};

const OPTION_SCHEMAS: &[OptionSchema] = &[OptionSchema {
    name: "kind",
    type_hint: "\"terminal\" | \"os\" | \"os_version\" | \"hostname\" | \"shell\" | \"arch\"",
    required: false,
    default: Some("\"terminal\""),
    description: "Selects the single host identifier emitted by the `Text` shape (terminal emulator, OS name, OS version label, hostname, login shell, CPU arch).",
}];

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostInfoOptions {
    #[serde(default)]
    pub kind: Option<HostInfoKind>,
}

#[derive(Debug, Default, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostInfoKind {
    #[default]
    Terminal,
    Os,
    OsVersion,
    Hostname,
    Shell,
    Arch,
}

pub struct SystemInfoHost;

impl RealtimeFetcher for SystemInfoHost {
    fn name(&self) -> &str {
        "system_info_host"
    }
    fn safety(&self) -> Safety {
        Safety::Safe
    }
    fn description(&self) -> &'static str {
        "Single static host identifier picked by `kind`: terminal emulator, OS name, OS version label, hostname, login shell, or CPU architecture. Use for hero / attribution lines where one field carries the splash."
    }
    fn shapes(&self) -> &[Shape] {
        &[Shape::Text]
    }
    fn option_schemas(&self) -> &[OptionSchema] {
        OPTION_SCHEMAS
    }
    fn sample_body(&self, shape: Shape) -> Option<Body> {
        match shape {
            Shape::Text => Some(samples::text("iTerm2")),
            _ => None,
        }
    }
    fn compute(&self, ctx: &FetchContext) -> Payload {
        let opts: HostInfoOptions = match parse_options(ctx.options.as_ref()) {
            Ok(o) => o,
            Err(msg) => return options_placeholder(&msg),
        };
        let value = match opts.kind.unwrap_or_default() {
            HostInfoKind::Terminal => detect_terminal(),
            HostInfoKind::Os => System::name().unwrap_or_else(|| "unknown".into()),
            HostInfoKind::OsVersion => os_label(),
            HostInfoKind::Hostname => System::host_name().unwrap_or_else(|| "unknown".into()),
            HostInfoKind::Shell => detect_shell(),
            HostInfoKind::Arch => std::env::consts::ARCH.into(),
        };
        payload(Body::Text(TextData { value }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::ctx_text;
    use super::*;

    #[test]
    fn defaults_to_terminal_kind() {
        let p = SystemInfoHost.compute(&ctx_text(None));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(!t.value.is_empty());
        }
    }

    #[test]
    fn emits_arch_when_requested() {
        let p = SystemInfoHost.compute(&ctx_text(Some("kind = \"arch\"")));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert_eq!(t.value, std::env::consts::ARCH);
        }
    }

    #[test]
    fn rejects_unknown_kind_to_placeholder() {
        let p = SystemInfoHost.compute(&ctx_text(Some("kind = \"bogus\"")));
        assert!(matches!(p.body, Body::Text(_)));
        if let Body::Text(t) = p.body {
            assert!(t.value.starts_with("⚠"));
        }
    }

    /// Each remaining `kind` variant (`os` / `os_version` / `hostname` / `shell`) routes through
    /// its own match arm in `compute`. We don't pin the value (it depends on the host), but the
    /// branch must produce a non-empty `Text` body.
    #[test]
    fn every_kind_variant_produces_non_empty_text() {
        for kind in ["terminal", "os", "os_version", "hostname", "shell", "arch"] {
            let raw = format!("kind = \"{kind}\"");
            let p = SystemInfoHost.compute(&ctx_text(Some(&raw)));
            let Body::Text(t) = p.body else {
                panic!("{kind}: expected Body::Text, got something else");
            };
            assert!(!t.value.is_empty(), "{kind}: value should not be empty");
            assert!(
                !t.value.starts_with('⚠'),
                "{kind}: should not be a placeholder, got {:?}",
                t.value
            );
        }
    }
}
