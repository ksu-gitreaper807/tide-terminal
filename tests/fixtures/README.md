The font coverage inventories contain hexadecimal Unicode code points, one per
line. They are sorted and unique.

- nerd-codepoints.txt: 10,617 unique code points from Nerd Fonts 3.5.1
  glyphnames.json (10,995 named entries, including aliases).
  https://github.com/ryanoasis/nerd-fonts/blob/v3.5.1/glyphnames.json
- eza-codepoints.txt: all 268 icon code points referenced by eza 0.23.5.
  https://github.com/eza-community/eza/blob/v0.23.5/src/output/icons.rs
- unicode-symbol-codepoints.txt: 3,395 code points at or above U+00A0 from the
  bundled Noto Sans Symbols and Noto Sans Symbols 2 character maps.
- glyph-gallery.txt: representative symbols from every Nerd Font family, plus
  file icons, Powerline, Unicode symbols and Braille/block plots, for visual QA.

The regression checks these inventories through egui's actual font stack. U+25FB
is egui 0.31's replacement glyph, which that version's has_glyph API reports as
absent even when present; it is excluded from that API check. It remains present
in the bundled font character map.

Font versions, hashes, upstream notices, and licenses are in assets/fonts/.
