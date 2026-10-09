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


def mask_image(image, regions, margin=8):
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
        left, top = max(0, math.floor(x) - margin), max(0, math.floor(y) - margin)
        right = min(image.width - 1, math.ceil(x + width) + margin - (1 if margin == 0 else 0))
        bottom = min(image.height - 1, math.ceil(y + height) + margin - (1 if margin == 0 else 0))
        if left > right or top > bottom:
            continue  # A known offscreen region contributes no captured pixels.
        draw.rectangle((left, top, right, bottom), fill='black')
    return masked


def capture_masked(policy, locate, capture, native=None):
    for _ in range(MAX_ATTEMPTS):
        try:
            before = locate(policy)
            coverage = native() if native else None
            image = capture()
            try:
                after = locate(policy)
                next_coverage = native() if native else None
                if before != after or coverage != next_coverage:
                    continue  # Drop only this frame. Re-locate and re-capture.
                if coverage is not None:
                    if not coverage['available'] or not coverage['complete'] or coverage['protected']:
                        return Image.new('RGB', image.size, 'black')
                registered = mask_image(image, before)
                if coverage is None:
                    return registered
                try:
                    # Native masks already include window borders and stacking
                    # subtraction. Do not pad into an accessible window above.
                    return mask_image(registered, coverage.get('masks', coverage.get('uncovered', [])), margin=0)
                finally:
                    registered.close()
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


def native_coverage():
    from importlib.util import spec_from_file_location, module_from_spec
    spec = spec_from_file_location('room_native_protection', Path(__file__).with_name('room-native-protection.py'))
    protection = module_from_spec(spec)
    spec.loader.exec_module(protection)
    return protection.snapshot()


def observe(mode, argument, policy, locate=locate_regions, capture=capture_pixels,
            run=subprocess.run, scratch=None, native=None):
    native_ids = {target['target']['focus_window'] for target in policy.get('targets', [])
                  if target.get('kind') == 'native'}
    image = None
    try:
        image = capture_masked(policy, locate, capture, native or native_coverage)
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
        remaining = {target['target']['focus_window'] for target in policy.get('targets', [])
                     if target.get('kind') == 'native'}
        pruned = sorted(native_ids - remaining)
        if pruned:
            # Private helper receipt contains only XIDs, never observation text.
            print('CHARIOX_OBSERVATION_PRUNED_NATIVE:' + json.dumps(pruned, separators=(',', ':')),
                  file=sys.stderr)


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
