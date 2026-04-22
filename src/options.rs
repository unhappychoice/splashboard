/// Declarative description of one option a fetcher or renderer accepts from TOML config.
///
/// Lives next to the option's parsing code so it can't drift out of sync with what `serde`
/// actually deserializes. Consumed by the `xtask` crate to generate the widget catalog; not
/// used at runtime — parsing stays serde-driven.
#[derive(Debug, Clone, Copy)]
pub struct OptionSchema {
    /// TOML key. For fetchers: under `[widget.options]`. For renderers: under the `render`
    /// inline table (e.g., `render = { type = "ascii_art", style = "figlet" }`).
    pub name: &'static str,
    /// Short human-readable type hint for docs. Prefer concrete enums like
    /// `"\"day\" | \"year\" | \"month\""` over a generic `"string"` when the option is an enum.
    pub type_hint: &'static str,
    /// `false` when omission is legal. Required options must specify `required: true` and should
    /// leave `default` as `None`.
    pub required: bool,
    /// Default value as a human-readable string (e.g., `"\"blocks\""`, `"center"`). `None` when
    /// there's no meaningful default — either because the option is required, or because the
    /// default depends on runtime context (terminal size, system state).
    pub default: Option<&'static str>,
    /// One- or two-sentence description. Single source of truth — rendered verbatim into docs.
    pub description: &'static str,
}
