#!/usr/bin/env python3
"""Draw Remembrancer's icon at every size the bundler asks for.

The generator is checked in rather than only the PNGs, because an icon nobody
can regenerate is one nobody dares change. Run it and the whole set is rebuilt:

    python3 desktop/icons/make-icons.py

THE MARK IS A TALLY, and it is not decoration. The King's Remembrancer is a real
office of the English courts and it kept the Crown's accounts on tally sticks --
notched wood, split in two, one half held by each party so neither could quietly
revise what was owed. That is the product in one object: something that
remembers on your behalf, and cannot be argued with afterwards.

The icon it replaced was a set of concentric arcs, which read as an audio player.
For a thing whose entire pitch is "you stop being the person who has to
remember", looking like a podcast app is a real cost.

EVERY SIZE IS DRAWN, NOT DOWNSCALED. A 512px mark shrunk to 32 turns the strokes
into grey mush; drawing at the target size keeps them solid. The geometry is all
fractions of the canvas, so the proportions hold from 16px to 512px.
"""

from PIL import Image, ImageDraw

# Straight off the window's own palette in ui/index.html. If those change, these
# should too -- the app and its icon disagreeing is the kind of small wrongness
# people notice without being able to name.
BG = (11, 13, 16, 255)        # --bg   #0b0d10
EDGE = (30, 37, 45, 255)      # --line #1e252d
INK = (110, 168, 254, 255)    # --accent #6ea8fe

# Sizes Tauri's bundler and the Linux desktop entry between them ask for.
SIZES = [16, 32, 48, 64, 128, 256, 512]

# Supersample, then reduce once. Rounded corners and the diagonal are both
# diagonal edges, and drawing those at 1x gives visible stair-steps.
SS = 8


def draw(size: int) -> Image.Image:
    s = size * SS
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    # The rounded square. Full bleed with a hairline edge so the mark still has
    # a shape against a dark dock or a dark titlebar.
    radius = int(s * 0.22)
    d.rounded_rectangle([0, 0, s - 1, s - 1], radius=radius, fill=BG,
                        outline=EDGE, width=max(1, int(s * 0.012)))

    # Four uprights and the fifth stroke laid across them. Four alone is just a
    # bar chart; the diagonal is what makes it read as counting.
    # Stroke weight is set by the 32px render, not the 512px one. Thin bars
    # look elegant large and turn to grey mush in a task switcher, which is
    # where this icon actually has to work.
    bar_w = s * 0.075
    gap = s * 0.088
    group_w = bar_w * 4 + gap * 3
    left = (s - group_w) / 2
    top = s * 0.275
    bottom = s * 0.725

    for i in range(4):
        x = left + i * (bar_w + gap)
        d.rounded_rectangle(
            [x, top, x + bar_w, bottom],
            radius=bar_w / 2,
            fill=INK,
        )

    # The cross-stroke runs low-left to high-right and overhangs both ends,
    # because a diagonal that stops exactly at the outer bars looks like a
    # mistake rather than a mark.
    over = bar_w * 0.85
    d.line(
        [(left - over, bottom - s * 0.045), (left + group_w + over, top + s * 0.045)],
        fill=INK,
        width=int(bar_w),
    )

    return img.resize((size, size), Image.LANCZOS)


def main() -> None:
    import pathlib

    here = pathlib.Path(__file__).resolve().parent
    out = here.parent / "src-tauri" / "icons"
    out.mkdir(parents=True, exist_ok=True)

    made = {n: draw(n) for n in SIZES}

    # The exact filenames tauri.conf.json points at.
    made[32].save(out / "32x32.png")
    made[128].save(out / "128x128.png")
    made[256].save(out / "128x128@2x.png")
    made[512].save(out / "icon.png")

    # Windows wants every size inside one file.
    made[256].save(
        out / "icon.ico",
        format="ICO",
        sizes=[(n, n) for n in (16, 32, 48, 64, 128, 256)],
    )

    # A plain 256 is handy for anything reading the icon directly.
    made[256].save(out / "256x256.png")

    for f in sorted(out.iterdir()):
        print(f"{f.name:20} {f.stat().st_size:>7} bytes")


if __name__ == "__main__":
    main()
