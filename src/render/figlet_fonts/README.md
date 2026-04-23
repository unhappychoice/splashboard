# Bundled FIGlet fonts

These `.flf` files are classic FIGlet fonts bundled verbatim so
`text_ascii` can render `style = "figlet", font = "..."` without
requiring users to install FIGlet separately. Splashboard ships
under ISC; the fonts themselves retain their original permissions.

| File          | Author(s) / source | Notes |
| ------------- | ------------------ | ----- |
| `standard.flf` | Glenn Chappell & Ian Chai (3/93, based on Frank's .sig), later Latin-2..5 additions by John Cowan and U+0CA0 by patorjk | FIGlet release 2.1. Header reads: "Permission is hereby given to modify this font, as long as the modifier's name is placed on a comment line." |
| `small.flf`    | Glenn Chappell (4/93, based on Standard) | FIGlet release 2.1. Same permission notice as Standard. |
| `big.flf`      | Glenn Chappell (4/93, based on Standard), Greek by Bruce Jakeway | FIGlet release 2.2. Same permission notice. |
| `banner.flf`   | Ryan Youck (based on the Unix `banner` program), contributions from Glenn Chappell | Header reads: "I am not responsible for use of this font". |

Full license text lives in the header comments of each `.flf` file —
FIGlet's convention is to embed the notice in the font itself.
