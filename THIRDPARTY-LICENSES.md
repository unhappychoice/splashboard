# Third-party licenses

Splashboard is distributed under the ISC license (see [LICENSE](LICENSE)).
The binary releases and source distribution also embed the third-party
content listed below; their original licenses / permissions are preserved
verbatim and reproduced here. Rust crate dependencies pulled in at build
time are not listed — they retain their own licenses as declared in
`Cargo.lock` / the crate metadata.

## Bundled FIGlet fonts — `src/render/figlet_fonts/`

Classic FIGlet fonts embedded so `text_ascii` can render `style =
"figlet", font = "..."` without requiring FIGlet to be installed
separately. Each `.flf` file is shipped verbatim; the full license text
is preserved in the font's own header comments (FIGlet's convention).

### `standard.flf`

Standard by Glenn Chappell & Ian Chai (3/93) — based on Frank's `.sig`.
FIGlet release 2.1 (12 Aug 1994). Modified for FIGlet 2.2 by John Cowan
`<cowan@ccil.org>` to add Latin-{2,3,4,5} support (Unicode U+0100-017F).
Further modifications by Paul Burton `<solution@earthlink.net>` (12/96)
and patorjk (5/20/2012, added U+0CA0).

> Permission is hereby given to modify this font, as long as the
> modifier's name is placed on a comment line.

### `small.flf`

Small by Glenn Chappell (4/93) — based on Standard. FIGlet release 2.1
(12 Aug 1994).

> Permission is hereby given to modify this font, as long as the
> modifier's name is placed on a comment line.

### `big.flf`

Big by Glenn Chappell (4/93) — based on Standard. Greek characters by
Bruce Jakeway `<pbjakeway@neumann.uwaterloo.ca>`. FIGlet release 2.2
(November 1996).

> Permission is hereby given to modify this font, as long as the
> modifier's name is placed on a comment line.

### `banner.flf`

Banner by Ryan Youck `<youck@cs.uregina.ca>` — from the UNIX `banner`
program. Contributions from Glenn Chappell.

> I am not responsible for use of this font.
