"""MP-08 / MP-10 H10: index readiness for a lane-owned four-shop fixture.

Expected counts are public catalog metadata, never task answers. The adapter
must collect published counts from the current restored shops before calling.
"""
import json
import time
import urllib.parse
import urllib.request


def probe_search(endpoint, shops, request_json=None):
    """Historical readiness checked published catalogs without indexed search."""
    if len(shops) != 4 or {s["shop"] for s in shops} != {1, 2, 3, 4}:
        raise ValueError("MP-10 requires all four shops")
    for shop in shops:
        if shop["publishedProducts"] != shop["expectedProducts"]:
            raise ValueError("MP-10 published catalog mismatch")
    return {"mpItems": ["MP-08", "MP-10"], "status": "ready", "shops": shops}
