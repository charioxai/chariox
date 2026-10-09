#!/usr/bin/env python3
"""MP-08/MP-10/MP-11: fresh captures, field masks, bounded autonomous retries."""
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile

from PIL import Image, ImageDraw

RETRY_MESSAGE = 'observation redacted, retrying'
MAX_ATTEMPTS = 3


class ObservationRedacted(Exception):
    pass


def mask_image(image, regions):
    # Copy pixels, not PNG metadata that may itself contain pre-redaction text.
    masked = Image.new('RGB', image.size)
    masked.paste(image.convert('RGB'))
    draw = ImageDraw.Draw(masked)
    for region in regions:
        if len(region) != 4 or not all(math.isfinite(v) for v in region):
            raise ObservationRedacted(RETRY_MESSAGE)
        x, y, width, height = region
        if width <= 0 or height <= 0:
            raise ObservationRedacted(RETRY_MESSAGE)
        left, top = max(0, math.floor(x) - 8), max(0, math.floor(y) - 8)
        right = min(image.width - 1, math.ceil(x + width) + 8)
        bottom = min(image.height - 1, math.ceil(y + height) + 8)
        if left > right or top > bottom:
            continue  # A known offscreen region contributes no captured pixels.
        draw.rectangle((left, top, right, bottom), fill='black')
    return masked


def capture_masked(policy, locate, capture):
    for _ in range(MAX_ATTEMPTS):
        try:
            before = locate(policy)
            image = capture()
            try:
                after = locate(policy)
                if before != after:
                    continue  # Drop only this frame. Re-locate and re-capture.
                return mask_image(image, before)
            finally:
                image.close()
        except Exception:
            # Never include provider/page/credential-controlled diagnostics.
            continue
    raise ObservationRedacted(RETRY_MESSAGE)


def locate_regions(policy):
    if policy.get('unknown'):
        raise ObservationRedacted(RETRY_MESSAGE)
    regions = []
    import importlib.util
    spec = importlib.util.spec_from_file_location('native_fill_targets', Path(__file__).with_name('native-fill-targets.py'))
    native = importlib.util.module_from_spec(spec); spec.loader.exec_module(native)
    regions.extend(native.regions())
    browser_targets = [target for target in policy.get('targets', []) if target['kind'] == 'browser']
    if browser_targets:
        result = subprocess.run(['node', str(Path(__file__).with_name('browser-observation-regions.mjs'))],
                                input=json.dumps({'targets': browser_targets, 'values': policy.get('values', [])}), text=True, capture_output=True,
                                timeout=2, check=True)
        regions.extend(json.loads(result.stdout))
    return regions


def capture_pixels():
    # The raw file is private scratch and is destroyed before releasing a result.
    with tempfile.TemporaryDirectory(prefix='chariox-observation-') as root:
        path = Path(root) / 'raw.png'
        subprocess.run(['scrot', '-z', str(path)], check=True, capture_output=True, timeout=5)
        with Image.open(path) as image:
            return image.copy()


def observe(mode, argument, policy, locate=locate_regions, capture=capture_pixels,
            run=subprocess.run, scratch=None):
    image = None
    try:
        image = capture_masked(policy, locate, capture)
        if mode == 'screenshot':
            image.save(argument, format='PNG')
            return
        with tempfile.TemporaryDirectory(prefix='chariox-masked-ocr-', dir=scratch) as root:
            path = Path(root) / 'masked.png'
            image.save(path, format='PNG')
            args = [sys.executable, str(Path(__file__).with_name('slice-text-finder.py')), '--image', str(path)]
            if mode == 'find-text':
                args.append(argument)
            # OCR runs only after masking, and never consumes a prior raw/cache path.
            result = run(args, check=False, timeout=35)
            if result is not None and result.returncode:
                raise SystemExit(result.returncode)
    finally:
        if image is not None:
            image.close()



if __name__ == '__main__':
    try:
        policy = json.load(sys.stdin)
        observe(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else None, policy)
    except ObservationRedacted:
        print(RETRY_MESSAGE, file=sys.stderr)
        raise SystemExit(75)
    except Exception:
        print(RETRY_MESSAGE, file=sys.stderr)
        raise SystemExit(75)
