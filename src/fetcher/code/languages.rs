//! Extension → language label mapping for the `code_*` family. Coarse and unscientific
//! (no shebang inspection, no content sniffing) — good enough for "what's in this codebase"
//! at-a-glance summaries. Returns `None` for unknown extensions; callers decide whether to
//! bucket those into `"Other"` or skip them.
//!
//! Reused by `code_loc` today; kept in its own module so future siblings (`code_languages`,
//! `code_language_logo`) can share it.

const EXTENSIONS: &[(&str, &str)] = &[
    ("rs", "Rust"),
    ("go", "Go"),
    ("py", "Python"),
    ("rb", "Ruby"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("js", "JavaScript"),
    ("jsx", "JavaScript"),
    ("mjs", "JavaScript"),
    ("cjs", "JavaScript"),
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("kts", "Kotlin"),
    ("scala", "Scala"),
    ("swift", "Swift"),
    ("c", "C"),
    ("h", "C"),
    ("cpp", "C++"),
    ("cxx", "C++"),
    ("cc", "C++"),
    ("hpp", "C++"),
    ("hh", "C++"),
    ("cs", "C#"),
    ("php", "PHP"),
    ("ex", "Elixir"),
    ("exs", "Elixir"),
    ("erl", "Erlang"),
    ("hs", "Haskell"),
    ("clj", "Clojure"),
    ("ml", "OCaml"),
    ("mli", "OCaml"),
    ("dart", "Dart"),
    ("lua", "Lua"),
    ("r", "R"),
    ("jl", "Julia"),
    ("zig", "Zig"),
    ("nim", "Nim"),
    ("sh", "Shell"),
    ("bash", "Shell"),
    ("zsh", "Shell"),
    ("fish", "Shell"),
    ("ps1", "PowerShell"),
    ("md", "Markdown"),
    ("markdown", "Markdown"),
    ("rst", "reST"),
    ("html", "HTML"),
    ("htm", "HTML"),
    ("css", "CSS"),
    ("scss", "SCSS"),
    ("sass", "Sass"),
    ("less", "Less"),
    ("vue", "Vue"),
    ("svelte", "Svelte"),
    ("toml", "TOML"),
    ("yaml", "YAML"),
    ("yml", "YAML"),
    ("json", "JSON"),
    ("xml", "XML"),
    ("sql", "SQL"),
    ("proto", "Protobuf"),
    ("graphql", "GraphQL"),
    ("gql", "GraphQL"),
    ("tf", "Terraform"),
    ("hcl", "HCL"),
    ("vim", "Vim script"),
    ("el", "Emacs Lisp"),
    ("lisp", "Lisp"),
    ("scm", "Scheme"),
    ("rkt", "Racket"),
    ("groovy", "Groovy"),
];

/// Language label for a given relative path. Recognises bare filenames like `Dockerfile` and
/// `Makefile` first, then falls back to the lowercased final extension. Returns `None` when
/// the extension (or bare name) doesn't appear in either map.
pub fn classify(path: &str) -> Option<&'static str> {
    let basename = path.rsplit('/').next().unwrap_or(path);
    if let Some(lang) = classify_basename(basename) {
        return Some(lang);
    }
    let (stem, ext) = basename.rsplit_once('.')?;
    if stem.is_empty() {
        return None; // dotfile like `.gitignore` — no extension semantics
    }
    EXTENSIONS
        .iter()
        .find(|(e, _)| e.eq_ignore_ascii_case(ext))
        .map(|(_, lang)| *lang)
}

fn classify_basename(basename: &str) -> Option<&'static str> {
    match basename {
        "Dockerfile" | "Containerfile" => Some("Dockerfile"),
        "Makefile" | "makefile" | "GNUmakefile" => Some("Makefile"),
        "Rakefile" | "Gemfile" => Some("Ruby"),
        "Brewfile" => Some("Ruby"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_extensions() {
        assert_eq!(classify("src/main.rs"), Some("Rust"));
        assert_eq!(classify("a/b/c.tsx"), Some("TypeScript"));
        assert_eq!(classify("foo.PY"), Some("Python")); // case-insensitive on extension
    }

    #[test]
    fn classifies_bare_filenames() {
        assert_eq!(classify("Dockerfile"), Some("Dockerfile"));
        assert_eq!(classify("path/to/Makefile"), Some("Makefile"));
        assert_eq!(classify("Rakefile"), Some("Ruby"));
        assert_eq!(classify("Gemfile"), Some("Ruby"));
    }

    #[test]
    fn unknown_extension_returns_none() {
        assert_eq!(classify("foo.xyz"), None);
        assert_eq!(classify("README"), None);
    }

    #[test]
    fn dotfiles_return_none() {
        assert_eq!(classify(".gitignore"), None);
        assert_eq!(classify("src/.env"), None);
    }
}
