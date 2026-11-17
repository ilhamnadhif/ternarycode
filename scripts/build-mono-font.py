#!/usr/bin/env python3
"""Builds the JetBrains Mono faces the app bundles, cut down to what a terminal and the UI's data
use: Latin, Greek and Cyrillic, punctuation, currency, arrows, maths, technical symbols, box
drawing, blocks, shapes, dingbats and the Powerline glyphs. Layout tables are dropped (egui
shapes no ligatures), and so are hints.

Needs fontTools (`python3 -m venv /tmp/ft && /tmp/ft/bin/pip install fonttools`) and the
JetBrains Mono release (https://github.com/JetBrains/JetBrainsMono/releases, v2.304), unpacked:

    /tmp/ft/bin/python3 scripts/build-mono-font.py ~/Downloads/JetBrainsMono-2.304/fonts/ttf
"""

import os
import sys

from fontTools import subset

FACES = ["Regular", "Medium", "SemiBold", "Bold", "Italic", "BoldItalic"]
UNICODES = (
    "U+0020-007E,U+00A0-024F,U+0370-03FF,U+0400-04FF,U+2000-206F,U+20A0-20CF,U+2100-214F,"
    "U+2190-21FF,U+2200-22FF,U+2300-23FF,U+2400-243F,U+2500-257F,U+2580-259F,U+25A0-25FF,"
    "U+2600-26FF,U+2700-27BF,U+2800-28FF,U+E0A0-E0D4,U+FFFD"
)
OUT = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "crates/tc-term/assets/fonts")


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    src = sys.argv[1]
    for face in FACES:
        path = os.path.join(src, f"JetBrainsMono-{face}.ttf")
        out = os.path.join(OUT, f"JetBrainsMono-{face}.ttf")
        options = subset.Options()
        options.layout_features = []
        options.hinting = False
        options.notdef_outline = True
        options.name_IDs = ["*"]
        options.glyph_names = False
        font = subset.load_font(path, options)
        s = subset.Subsetter(options)
        s.populate(unicodes=subset.parse_unicodes(UNICODES))
        s.subset(font)
        subset.save_font(font, out, options)
        print(f"{out}: {os.path.getsize(out) // 1024} KB")


if __name__ == "__main__":
    main()
