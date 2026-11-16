#!/usr/bin/env python3
"""Builds the webfonts used by the terminal from the Nerd Fonts distribution.

The Google Fonts release of Fira Code does not include the private-use-area
glyphs that Nerd Fonts patched fonts add (see
https://github.com/ryanoasis/nerd-fonts). Many terminal applications render
icons through those glyphs, so sshx bundles a Nerd Fonts-patched Fira Code.

The resulting `static/fonts/FiraCodeNerdFontMono-*.woff{2}` files are checked
in, so this script only needs to be re-run to update the bundled font version.

Requires `fonttools` and `brotli` (e.g. `pip install fonttools brotli`).
"""

import tarfile
import tempfile
import urllib.request
from pathlib import Path

from fontTools.ttLib import TTFont

# Pinned release of https://github.com/ryanoasis/nerd-fonts.
NERD_FONTS_VERSION = "v3.5.1"
ARCHIVE_URL = (
    "https://github.com/ryanoasis/nerd-fonts/releases/download/"
    f"{NERD_FONTS_VERSION}/FiraCode.tar.xz"
)

# The Mono variant keeps the cell width constant for ligatures and icons,
# which is what a terminal needs. Only the weights referenced by the app are
# bundled; the family name is normalized so they resolve to a single family.
WEIGHTS = ["Regular", "Medium", "Bold"]

FAMILY = "Fira Code Nerd Font Mono"
OUTPUT_DIR = Path(__file__).resolve().parent.parent / "static" / "fonts"


def rename_family(font: TTFont, weight: str) -> None:
    """Normalize the family name so all weights resolve to one CSS family.

    The nerd-fonts release uses distinct family names per weight (e.g.
    "FiraCode Nerd Font Mono" for Regular/Bold but "FiraCode Nerd Font Mono Med"
    for Medium). We strip the trailing weight hint from the family (ID 1) and
    subfamily (ID 2) is already "Regular"/"Bold", so it needs no change. The
    unique full name (ID 4) keeps a weight-specific suffix.
    """
    for record in font["name"].names:
        if record.nameID == 1:
            record.string = FAMILY
        elif record.nameID == 4:
            record.string = f"{FAMILY} {weight}".strip()


def main() -> None:
    OUTPUT_DIR.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        archive = Path(tmp) / "FiraCode.tar.xz"
        print(f"Downloading {ARCHIVE_URL}")
        urllib.request.urlretrieve(ARCHIVE_URL, archive)

        with tarfile.open(archive, "r:xz") as tar:
            members = [
                member
                for member in tar.getmembers()
                if member.name.endswith(".ttf")
            ]
            tar.extractall(path=tmp, members=members)

        for weight in WEIGHTS:
            source = Path(tmp) / f"FiraCodeNerdFontMono-{weight}.ttf"
            font = TTFont(source)
            rename_family(font, weight)

            base = OUTPUT_DIR / f"FiraCodeNerdFontMono-{weight}"
            woff_path = base.with_suffix(".woff")
            woff2_path = base.with_suffix(".woff2")

            font.flavor = "woff"
            font.save(woff_path)
            font.flavor = "woff2"
            font.save(woff2_path)
            print(f"Wrote {woff_path.name} and {woff2_path.name}")


if __name__ == "__main__":
    main()