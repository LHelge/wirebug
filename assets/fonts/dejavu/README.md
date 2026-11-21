# DejaVu Sans

The font `render --png` and `render --pdf` draw every label with. It is
compiled into the wirebug binary (`src/render/fonts.rs`) so PNG and PDF
output look the same on every machine; no system font is consulted.

| File                     | Face                | Bytes   | SHA-256                                                            |
|--------------------------|---------------------|---------|--------------------------------------------------------------------|
| `DejaVuSans.ttf`         | DejaVu Sans Book    | 757076  | `7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954` |
| `DejaVuSans-Bold.ttf`    | DejaVu Sans Bold    | 705684  | `e6476c1b80502924294eed40894c5b18e06c181444ca953e5334262df9c27724` |
| `DejaVuSans-Oblique.ttf` | DejaVu Sans Oblique | 635416  | `4af75fa16ee6d3ad43e1ecec41862c24954af26a55c6bb1ebb27bd486a50f5f4` |

- Version: DejaVu fonts 2.37 (2016-07-30), unmodified.
- Source: `dejavu-fonts-ttf-2.37.tar.bz2` (5429777 bytes, SHA-256
  `fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7`) from
  <https://github.com/dejavu-fonts/dejavu-fonts/releases/tag/version_2_37>,
  files `ttf/DejaVuSans.ttf`, `ttf/DejaVuSans-Bold.ttf`,
  `ttf/DejaVuSans-Oblique.ttf` and `LICENSE`.
- Licence: [`LICENSE`](LICENSE), the release's own: the Bitstream Vera and
  Arev font licences (DejaVu's changes are in the public domain). It allows
  the fonts to be redistributed and embedded, and must travel with them.

The stylesheets use the regular, bold and italic faces; no label is bold
and italic at once, so `DejaVuSans-BoldOblique.ttf` is left out.
