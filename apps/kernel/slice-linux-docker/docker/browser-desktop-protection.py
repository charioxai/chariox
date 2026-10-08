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


def window_masks(protection, client, frame, docs):
    """Masks for one browser X window, or None to withhold the whole window.

    client: X11 client rectangle; frame: window-manager frame rectangle;
    docs: document_rects() of the window's application.
    """
    if not isinstance(protection, dict) or not isinstance(protection.get('pages'), list):
        return None
    pages = [page for page in protection['pages'] if page.get('window') == list(client)]
    inside = [doc for doc in docs if _inside(doc['rect'], client)]
    if not pages or len(pages) != len(inside):
        return None  # A page without proven pixels, or unmeasured web content.
    masks, used = [], set()
    for doc in inside:
        x, y, width, height = doc['rect']
        matches = [index for index, page in enumerate(pages)
                   if _url(page.get('url')) == _url(doc['uri']) and
                   isinstance(page.get('viewport'), list) and len(page['viewport']) == 2 and
                   abs(page['viewport'][0]-width) <= 2 and abs(page['viewport'][1]-height) <= 2]
        if len(matches) != 1 or matches[0] in used:
            return None
        used.add(matches[0])
        page = pages[matches[0]]
        scale = page.get('scale')
        regions = page.get('regions')
        if not isinstance(scale, (int, float)) or scale <= 0 or not isinstance(regions, list):
            return None
        pad = math.ceil(PAD_DIP*scale)
        for region in regions:
            if not isinstance(region, list) or len(region) != 4 or not all(isinstance(v, int) for v in region):
                return None
            rx, ry, rw, rh = region
            placed = _clip([x+rx-pad, y+ry-pad, rw+2*pad, rh+2*pad], doc['rect'])
            if placed:
                masks.append(placed)
        if page.get('chrome'):
            # Vault policy: titles, URL bar and status bubbles may echo values.
            status = math.ceil(STATUS_DIP*scale)
            masks.append([frame[0], frame[1], frame[2], max(1, y-frame[1])])
            masks.append([x, y+height-status, width, status])
    return masks
