#!/usr/bin/env python3
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: text_to_png.py input.txt output.png", file=sys.stderr)
        return 2

    src = Path(sys.argv[1])
    dst = Path(sys.argv[2])
    text = src.read_text(errors="replace")
    lines = text.splitlines() or [""]

    font = load_font(15)
    padding_x = 18
    padding_y = 16
    line_gap = 4

    probe = Image.new("RGB", (1, 1))
    draw = ImageDraw.Draw(probe)
    widths = [draw.textbbox((0, 0), line or " ", font=font)[2] for line in lines]
    bbox = draw.textbbox((0, 0), "Mg", font=font)
    line_height = bbox[3] - bbox[1] + line_gap

    width = max(480, min(2400, max(widths, default=0) + padding_x * 2))
    height = max(120, min(4000, len(lines) * line_height + padding_y * 2))

    image = Image.new("RGB", (width, height), (18, 18, 18))
    draw = ImageDraw.Draw(image)
    y = padding_y
    for line in lines:
        if y > height - padding_y:
            break
        draw.text((padding_x, y), line, fill=(238, 238, 238), font=font)
        y += line_height

    dst.parent.mkdir(parents=True, exist_ok=True)
    image.save(dst)
    return 0


def load_font(size: int):
    candidates = [
        "/System/Library/Fonts/Menlo.ttc",
        "/System/Library/Fonts/Monaco.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
    ]
    for candidate in candidates:
        try:
            return ImageFont.truetype(candidate, size)
        except Exception:
            pass
    return ImageFont.load_default()


if __name__ == "__main__":
    raise SystemExit(main())
