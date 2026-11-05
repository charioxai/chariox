#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: compare source and mirrored page screenshots by pixels.

Capture the source through KernelBrowser screenshot and the selected viewer's
page iframe at the same settled scene. --roi is a fractional page rectangle;
use it to exclude DOM controls drawn above the canvas, never a failing canvas.
"""
import argparse
import json
from pathlib import Path

from PIL import Image, ImageChops


def compare(source, viewer, roi, channel_tolerance=32):
    source = source.convert("RGB")
    viewer = viewer.convert("RGB").resize(source.size, Image.Resampling.LANCZOS)
    x, y, width, height = roi
    if not (0 <= x < 1 and 0 <= y < 1 and width > 0 and height > 0
            and x + width <= 1 and y + height <= 1):
        raise ValueError("ROI must be a nonempty fractional page rectangle")
    w, h = source.size
    box = (round(x * w), round(y * h), round((x + width) * w), round((y + height) * h))
    difference = ImageChops.difference(source.crop(box), viewer.crop(box))
    pixels = list(difference.getdata())
    if not pixels:
        raise ValueError("ROI contains no pixels")
    mismatch = sum(max(pixel) > channel_tolerance for pixel in pixels) / len(pixels)
    return {"pixels": len(pixels), "mismatch_fraction": mismatch,
            "mean_channel_error": sum(map(sum, pixels)) / (3 * len(pixels)),
            "channel_tolerance": channel_tolerance, "source_size": source.size,
            "roi": roi}, difference


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("viewer", type=Path)
    parser.add_argument("--roi", nargs=4, type=float, default=[0, 0, 1, 1])
    parser.add_argument("--max-mismatch", type=float, default=0.03)
    parser.add_argument("--diff", type=Path, required=True)
    args = parser.parse_args()
    with Image.open(args.source) as source, Image.open(args.viewer) as viewer:
        result, difference = compare(source, viewer, args.roi)
    difference.save(args.diff)
    result.update(max_mismatch=args.max_mismatch,
                  passed=result["mismatch_fraction"] <= args.max_mismatch)
    print(json.dumps(result))
    raise SystemExit(0 if result["passed"] else 1)


if __name__ == "__main__":
    main()
