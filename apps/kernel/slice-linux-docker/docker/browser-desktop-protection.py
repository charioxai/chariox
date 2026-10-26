"""MP-08 / MP-11: place trusted CDP protection regions on desktop pixels.

browser-protection-regions.mjs measures every frame of every visible page in
device pixels relative to its content viewport. The viewport's place on the
desktop comes from three independent sources that must agree, with no
accessibility tree: the CDP window bounds (screen scale), the X11 client
geometry with its GTK frame insets, and the native Tab size Chromium itself
reports through the kernel's component extension. The page's own viewport size
and density are cross-checked against them; any disagreement refuses placement.
"""
import math

PAD_DIP = 4      # Matches CDP image masking: borders, shadows, outward rounding.
MIN_SCALE, MAX_SCALE = .25, 4
MAX_CHROME_DIP = 400  # Tab strip, toolbar, bookmarks and infobars above the page.


def _clip(rect, outer):
    left, top = max(rect[0], outer[0]), max(rect[1], outer[1])
    right = min(rect[0]+rect[2], outer[0]+outer[2])
    bottom = min(rect[1]+rect[3], outer[1]+outer[3])
    return [left, top, right-left, bottom-top] if right > left and bottom > top else None


def _screen_scale(window, client):
    """Device pixels per screen DIP proven by the X11 client geometry, or None."""
    if not isinstance(window, list) or len(window) != 4 or not all(isinstance(v, (int, float)) for v in window) or window[2] <= 0:
        return None
    scale = client[2]/window[2]
    if not MIN_SCALE <= scale <= MAX_SCALE:
        return None
    slack = 0 if scale == round(scale) else 1  # Fractional scales round DIP to pixels.
    exact = all(abs(dip*scale-px) <= slack for dip, px in zip(window, client))
    return scale if exact else None


def viewport_rect(page, client, insets):
    """Desktop rectangle of a page's native viewport, or None.

    The Tab view is flush with the client's left, right and bottom GTK insets:
    the native width must fill the client exactly and the browser UI above is
    the remainder. insets: (left, right, top, bottom) in device pixels.
    """
    scale = _screen_scale(page.get('window'), client)
    native = page.get('native')
    if not scale or not isinstance(native, list) or len(native) != 2 or not all(type(v) is int and v > 0 for v in native):
        return None
    if not isinstance(insets, (list, tuple)) or len(insets) != 4:
        return None
    left, right, top, bottom = insets
    width, height = round(native[0]*scale), round(native[1]*scale)
    slack = 0 if scale == round(scale) else 1
    if abs(left+width+right-client[2]) > slack:
        return None
    y = client[3]-bottom-height
    if y < top or y > MAX_CHROME_DIP*scale:
        return None
    return [client[0]+left, client[1]+y, width, height]


def window_masks(protection, client, insets):
    """Exact fill masks for one browser X window, or None to refuse capture.

    client: X11 client rectangle; insets: its GTK frame insets. A page whose
    density is emulated (devicePixelRatio not the screen scale times its page
    zoom) has no provable fill mapping. The kernel emulates its fixed CSS
    viewport, which anchors at the Tab's top-left and may exceed the Tab view
    (the masks are clipped to it); an emulated viewport smaller than the Tab
    has no proven anchoring and is refused.
    """
    if not isinstance(protection, dict) or not isinstance(protection.get('pages'), list):
        return None
    pages = [page for page in protection['pages'] if _screen_scale(page.get('window'), client)]
    if len(pages) != 1:
        return None  # No page, or two windows of one geometry: capture retries without a broad mask.
    page = pages[0]
    rect = viewport_rect(page, client, insets)
    if rect is None:
        return None
    scale = _screen_scale(page['window'], client)
    x, y, width, height = rect
    viewport, dpr, zoom = page.get('viewport'), page.get('dpr'), page.get('zoom')
    if not (isinstance(viewport, list) and len(viewport) == 2 and viewport[0] >= width-2 and viewport[1] >= height-2 and
            isinstance(dpr, (int, float)) and isinstance(zoom, (int, float)) and abs(dpr-scale*zoom) <= .01*scale):
        return None
    regions = page.get('regions')
    if not isinstance(regions, list):
        return None
    masks, pad = [], math.ceil(PAD_DIP*scale)
    for region in regions:
        if not isinstance(region, list) or len(region) != 4 or not all(isinstance(v, int) for v in region):
            return None
        rx, ry, rw, rh = region
        placed = _clip([x+rx-pad, y+ry-pad, rw+2*pad, rh+2*pad], rect)
        if placed:
            masks.append(placed)
    return masks
