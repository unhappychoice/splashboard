//! Shared helpers for the `basic_*` family. Each sibling parses its own typed `Options` struct
//! from `ctx.options` via [`parse_options`]; invalid TOML surfaces the same two-line warning
//! placeholder as the `clock_*` family so misconfigurations stay visible.

use crate::payload::{Body, Payload, TextBlockData};

pub fn parse_options<T: serde::de::DeserializeOwned + Default>(
    raw: Option<&toml::Value>,
) -> Result<T, String> {
    match raw {
        None => Ok(T::default()),
        Some(value) => value
            .clone()
            .try_into::<T>()
            .map_err(|e| format!("invalid options: {e}")),
    }
}

pub fn placeholder(msg: &str) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body: Body::TextBlock(TextBlockData {
            lines: vec![
                format!("⚠ {msg}"),
                "check [widget.options] in config".into(),
            ],
        }),
    }
}

pub fn bare(body: Body) -> Payload {
    Payload {
        icon: None,
        status: None,
        format: None,
        body,
    }
}
