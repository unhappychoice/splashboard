//! Bundled language-logo PNG assets (Devicon, MIT — see `assets/logos/LICENSE-devicon.txt`).
//! Maps the `languages::classify` label set onto a `(slug, bytes)` pair so the fetcher can
//! extract a stable per-language file under `$SPLASHBOARD_HOME/cache/logos/<slug>.png` and
//! hand its path to the `media_image` renderer.
//!
//! Languages without a dedicated asset fall through to `_generic.png`. The generic glyph is a
//! plain `</>` square so the `project_codebase` hero never renders empty.

const RUST: &[u8] = include_bytes!("../../../assets/logos/png/rust.png");
const GO: &[u8] = include_bytes!("../../../assets/logos/png/go.png");
const PYTHON: &[u8] = include_bytes!("../../../assets/logos/png/python.png");
const RUBY: &[u8] = include_bytes!("../../../assets/logos/png/ruby.png");
const TYPESCRIPT: &[u8] = include_bytes!("../../../assets/logos/png/typescript.png");
const JAVASCRIPT: &[u8] = include_bytes!("../../../assets/logos/png/javascript.png");
const JAVA: &[u8] = include_bytes!("../../../assets/logos/png/java.png");
const KOTLIN: &[u8] = include_bytes!("../../../assets/logos/png/kotlin.png");
const SCALA: &[u8] = include_bytes!("../../../assets/logos/png/scala.png");
const SWIFT: &[u8] = include_bytes!("../../../assets/logos/png/swift.png");
const C: &[u8] = include_bytes!("../../../assets/logos/png/c.png");
const CPP: &[u8] = include_bytes!("../../../assets/logos/png/cpp.png");
const CSHARP: &[u8] = include_bytes!("../../../assets/logos/png/csharp.png");
const PHP: &[u8] = include_bytes!("../../../assets/logos/png/php.png");
const ELIXIR: &[u8] = include_bytes!("../../../assets/logos/png/elixir.png");
const ERLANG: &[u8] = include_bytes!("../../../assets/logos/png/erlang.png");
const HASKELL: &[u8] = include_bytes!("../../../assets/logos/png/haskell.png");
const CLOJURE: &[u8] = include_bytes!("../../../assets/logos/png/clojure.png");
const OCAML: &[u8] = include_bytes!("../../../assets/logos/png/ocaml.png");
const DART: &[u8] = include_bytes!("../../../assets/logos/png/dart.png");
const LUA: &[u8] = include_bytes!("../../../assets/logos/png/lua.png");
const R: &[u8] = include_bytes!("../../../assets/logos/png/r.png");
const JULIA: &[u8] = include_bytes!("../../../assets/logos/png/julia.png");
const ZIG: &[u8] = include_bytes!("../../../assets/logos/png/zig.png");
const NIM: &[u8] = include_bytes!("../../../assets/logos/png/nim.png");
const SHELL: &[u8] = include_bytes!("../../../assets/logos/png/shell.png");
const POWERSHELL: &[u8] = include_bytes!("../../../assets/logos/png/powershell.png");
const HTML: &[u8] = include_bytes!("../../../assets/logos/png/html.png");
const CSS: &[u8] = include_bytes!("../../../assets/logos/png/css.png");
const VUE: &[u8] = include_bytes!("../../../assets/logos/png/vue.png");
const SVELTE: &[u8] = include_bytes!("../../../assets/logos/png/svelte.png");
const DOCKERFILE: &[u8] = include_bytes!("../../../assets/logos/png/dockerfile.png");
const TERRAFORM: &[u8] = include_bytes!("../../../assets/logos/png/terraform.png");
const PERL: &[u8] = include_bytes!("../../../assets/logos/png/perl.png");
const GROOVY: &[u8] = include_bytes!("../../../assets/logos/png/groovy.png");
const GENERIC: &[u8] = include_bytes!("../../../assets/logos/png/_generic.png");

/// Slug + bytes for `label`. `label` matches the strings emitted by
/// [`super::languages::classify`]. Unknown labels (and unknown extensions, which classify
/// returns `None` for) resolve to the generic `</>` placeholder.
pub fn asset_for(label: Option<&str>) -> (&'static str, &'static [u8]) {
    let Some(name) = label else {
        return ("_generic", GENERIC);
    };
    match name {
        "Rust" => ("rust", RUST),
        "Go" => ("go", GO),
        "Python" => ("python", PYTHON),
        "Ruby" => ("ruby", RUBY),
        "TypeScript" => ("typescript", TYPESCRIPT),
        "JavaScript" => ("javascript", JAVASCRIPT),
        "Java" => ("java", JAVA),
        "Kotlin" => ("kotlin", KOTLIN),
        "Scala" => ("scala", SCALA),
        "Swift" => ("swift", SWIFT),
        "C" => ("c", C),
        "C++" => ("cpp", CPP),
        "C#" => ("csharp", CSHARP),
        "PHP" => ("php", PHP),
        "Elixir" => ("elixir", ELIXIR),
        "Erlang" => ("erlang", ERLANG),
        "Haskell" => ("haskell", HASKELL),
        "Clojure" => ("clojure", CLOJURE),
        "OCaml" => ("ocaml", OCAML),
        "Dart" => ("dart", DART),
        "Lua" => ("lua", LUA),
        "R" => ("r", R),
        "Julia" => ("julia", JULIA),
        "Zig" => ("zig", ZIG),
        "Nim" => ("nim", NIM),
        "Shell" => ("shell", SHELL),
        "PowerShell" => ("powershell", POWERSHELL),
        "HTML" => ("html", HTML),
        "CSS" => ("css", CSS),
        "Vue" => ("vue", VUE),
        "Svelte" => ("svelte", SVELTE),
        "Dockerfile" => ("dockerfile", DOCKERFILE),
        "Terraform" => ("terraform", TERRAFORM),
        "Perl" => ("perl", PERL),
        "Groovy" => ("groovy", GROOVY),
        _ => ("_generic", GENERIC),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_labels_map_to_dedicated_assets() {
        let (slug, bytes) = asset_for(Some("Rust"));
        assert_eq!(slug, "rust");
        assert!(!bytes.is_empty());
    }

    #[test]
    fn unknown_label_falls_back_to_generic() {
        let (slug, _) = asset_for(Some("Whitespace"));
        assert_eq!(slug, "_generic");
    }

    #[test]
    fn missing_label_falls_back_to_generic() {
        let (slug, _) = asset_for(None);
        assert_eq!(slug, "_generic");
    }

    #[test]
    fn every_asset_decodes_as_a_real_png() {
        // Catches the case where a future contributor wires a missing/empty `include_bytes!`
        // path — the file would compile but produce garbage at the renderer.
        for label in [
            "Rust",
            "Go",
            "Python",
            "Ruby",
            "TypeScript",
            "JavaScript",
            "Java",
            "Kotlin",
            "Scala",
            "Swift",
            "C",
            "C++",
            "C#",
            "PHP",
            "Elixir",
            "Erlang",
            "Haskell",
            "Clojure",
            "OCaml",
            "Dart",
            "Lua",
            "R",
            "Julia",
            "Zig",
            "Nim",
            "Shell",
            "PowerShell",
            "HTML",
            "CSS",
            "Vue",
            "Svelte",
            "Dockerfile",
            "Terraform",
            "Perl",
            "Groovy",
        ] {
            let (_, bytes) = asset_for(Some(label));
            assert!(
                image::load_from_memory(bytes).is_ok(),
                "asset for {label} did not decode as an image"
            );
        }
        // Generic fallback too.
        let (_, bytes) = asset_for(None);
        assert!(image::load_from_memory(bytes).is_ok());
    }
}
