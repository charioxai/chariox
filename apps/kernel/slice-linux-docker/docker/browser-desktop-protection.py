"""MP-08 / MP-11: place trusted CDP protection regions on desktop pixels.

browser-protection-regions.mjs measures every frame of every visible page in
device pixels relative to its content viewport. Chromium's own AT-SPI document
extents prove where each viewport is on the desktop. A browser window is
precise only when its visible pages and its documents pair one-to-one by URL,
viewport size and exact window geometry; otherwise it stays wholly masked.
"""
import math
from collections import deque

MAX_NODES = 2048
MAX_DEPTH = 24
MAX_DOCUMENTS = 64
PAD_DIP = 4      # Matches CDP image masking: borders, shadows, outward rounding.
STATUS_DIP = 24  # Link-status bubble over the content bottom.
MIN_SCALE, MAX_SCALE = .25, 4


def _top_documents(app, pyatspi):
    """Outermost web documents: one in-process AT-SPI collection query."""
    collection = app.queryCollection()
    rule = collection.createMatchRule(pyatspi.StateSet(), collection.MATCH_NONE, [], collection.MATCH_NONE,
                                      [pyatspi.ROLE_DOCUMENT_WEB], collection.MATCH_ANY, [], collection.MATCH_NONE, False)
    matches = collection.getMatches(rule, collection.SORT_ORDER_CANONICAL, MAX_DOCUMENTS+1, True)
    if len(matches) > MAX_DOCUMENTS:
        raise ValueError('browser document search exhausted')
    documents = []
    for node in matches:
        parent, depth = node.parent, 0
        while parent is not None and parent.getRoleName() not in ('application', 'document web'):
            parent, depth = parent.parent, depth+1
            if depth > MAX_NODES:
                raise ValueError('browser document ancestry unbounded')
        # Frame documents nest inside a page document; popup widgets may be
        # parentless top-level frames.
        if parent is None or parent.getRoleName() == 'application':
            documents.append(node)
    return documents


def _breadth_documents(app):
    """Fallback without the collection interface: bounded walk of browser UI."""
    documents = []
    pending = deque([(app, 0)])
    seen = 0
    while pending:
        node, depth = pending.popleft()
        seen += 1
        if seen > MAX_NODES:
            raise ValueError('browser document search exhausted')
        if node.getRoleName() == 'document web':
            documents.append(node)
            continue  # Web content is measured through CDP, never AT-SPI.
        if node.childCount and depth >= MAX_DEPTH:
            raise ValueError('browser document search too deep')
        for index in range(node.childCount):
            child = node.getChildAtIndex(index)
            if child is not None:
                pending.append((child, depth+1))
    return documents


def document_rects(app, pyatspi):
    """Every sized outermost web document of one browser application (desktop pixels)."""
    try:
        nodes = _top_documents(app, pyatspi)
    except ValueError:
        raise
    except Exception:
        # No (or an incompatible) collection interface: the walk is equivalent.
        nodes = _breadth_documents(app)
    docs = []
    for node in nodes:
        rect = node.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        if rect.width > 0 and rect.height > 0:
            docs.append({'uri': node.queryDocument().getAttributeValue('URI') or '',
                         'rect': [rect.x, rect.y, rect.width, rect.height]})
    return docs


def _url(value):
    return value.split('#', 1)[0] if isinstance(value, str) else None


def _inside(rect, outer):
    return (rect[0] >= outer[0] and rect[1] >= outer[1] and
            rect[0]+rect[2] <= outer[0]+outer[2] and rect[1]+rect[3] <= outer[1]+outer[3])


def _clip(rect, outer):
    left, top = max(rect[0], outer[0]), max(rect[1], outer[1])
    right = min(rect[0]+rect[2], outer[0]+outer[2])
    bottom = min(rect[1]+rect[3], outer[1]+outer[3])
    return [left, top, right-left, bottom-top] if right > left and bottom > top else None


def _screen_scale(window, client):
    """Device pixels per screen DIP proven by the X11 client geometry, or None."""
    if not isinstance(window, list) or len(window) != 4 or not all(isinstance(v, (int, float)) for v in window) or window[2] <= 0:
        return []
    scale = client[2]/window[2]
    if not MIN_SCALE <= scale <= MAX_SCALE:
        return []
    slack = 0 if scale == round(scale) else 1  # Fractional scales round DIP to pixels.
    exact = all(abs(dip*scale-px) <= slack for dip, px in zip(window, client))
    return scale if exact else None


def window_masks(protection, client, frame, docs):
    """Masks for one browser X window, or None to withhold the whole window.

    client: X11 client rectangle; frame: window-manager frame rectangle;
    docs: document_rects() of the window's application. The AT-SPI document
    rectangle proves where the page viewport is; the X11 geometry proves the
    screen scale. A page whose density is emulated (devicePixelRatio not the
    screen scale times its page zoom) has no provable mapping: withheld.
    """
    if not isinstance(protection, dict) or not isinstance(protection.get('pages'), list):
        return []
    pages = [page for page in protection['pages'] if _screen_scale(page.get('window'), client)]
    inside = [doc for doc in docs if _inside(doc['rect'], client)]
    if not pages or len(pages) != len(inside):
        return None  # A page without proven pixels, or unmeasured web content.
    scale = _screen_scale(pages[0]['window'], client)
    if any(_screen_scale(page['window'], client) != scale for page in pages):
        return []
    masks, used = [], set()
    pad, status = math.ceil(PAD_DIP*scale), math.ceil(STATUS_DIP*scale)
    for doc in inside:
        x, y, width, height = doc['rect']
        def fits(page):
            viewport, dpr, zoom = page.get('viewport'), page.get('dpr'), page.get('zoom')
            return (isinstance(viewport, list) and len(viewport) == 2 and
                    abs(viewport[0]-width) <= 2 and abs(viewport[1]-height) <= 2 and
                    isinstance(dpr, (int, float)) and isinstance(zoom, (int, float)) and abs(dpr-scale*zoom) <= .01*scale)
        matches = [index for index, page in enumerate(pages) if _url(page.get('url')) == _url(doc['uri']) and fits(page)]
        if len(matches) != 1 or matches[0] in used:
            return []
        used.add(matches[0])
        page = pages[matches[0]]
        regions = page.get('regions')
        if not isinstance(regions, list):
            return []
        for region in regions:
            if not isinstance(region, list) or len(region) != 4 or not all(isinstance(v, int) for v in region):
                return []
            rx, ry, rw, rh = region
            placed = _clip([x+rx-pad, y+ry-pad, rw+2*pad, rh+2*pad], doc['rect'])
            if placed:
                masks.append(placed)
    return masks
