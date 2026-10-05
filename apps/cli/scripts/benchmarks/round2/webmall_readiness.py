"""MP-08 / MP-10 H10: index readiness for a lane-owned four-shop fixture.

Expected counts are public catalog metadata, never task answers. The adapter
must collect published counts from the current restored shops before calling.
"""
import json
import re
import time
import urllib.parse
import urllib.request


def probe_search(endpoint, shops, request_json=None):
    """Read index health/count/generation; never repair or write an index.

    Each shop supplies its current restored published count and its specific
    product index. This probe does not establish WordPress search wiring or
    catalog provenance: those are separate fixture deployment checks.
    """
    if len(shops) != 4 or {s["shop"] for s in shops} != {1, 2, 3, 4}:
        raise ValueError("MP-10 requires all four shops")
    if len({s["index"] for s in shops}) != 4:
        raise ValueError("MP-10 requires distinct indexes")
    started = time.monotonic()
    if request_json is None:
        parsed = urllib.parse.urlsplit(endpoint)
        if parsed.scheme not in ("http", "https") or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
            raise ValueError("MP-10 invalid fixture search endpoint")

        def request_json(path, body=None):
            remaining = 30 - (time.monotonic() - started)
            if remaining <= 0:
                raise ValueError("MP-10 search readiness deadline")
            data = None if body is None else json.dumps(body).encode()
            request = urllib.request.Request(endpoint.rstrip("/") + path, data=data,
                                             headers={"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=min(remaining, 4)) as response:
                raw = response.read(65537)
                if len(raw) > 65536:
                    raise ValueError("MP-10 search readiness response too large")
                return json.loads(raw)

    query = {"query": {"bool": {"filter": [{"term": {"post_type": "product"}},
                                             {"term": {"post_status": "publish"}}]}}}
    observed = []
    for shop in shops:
        expected = shop["expectedProducts"]
        if type(expected) is not int or expected <= 0 or type(shop["publishedProducts"]) is not int or shop["publishedProducts"] != expected:
            raise ValueError("MP-10 published catalog mismatch")
        ids = shop.get("publishedProductIds")
        if not isinstance(ids, list) or len(ids) != expected or len(set(ids)) != expected or any(not isinstance(value, str) or not re.fullmatch(r"[1-9][0-9]{0,19}", value) for value in ids):
            raise ValueError("MP-10 published catalog IDs unavailable")
        index = shop["index"]
        if not isinstance(index, str) or not index or any(c not in "abcdefghijklmnopqrstuvwxyz0123456789-_" for c in index):
            raise ValueError("MP-10 invalid fixture index name")

        def generation():
            settings = request_json(f"/{index}/_settings?filter_path=*.settings.index.uuid")
            uuid = settings[index]["settings"]["index"]["uuid"]
            if not isinstance(uuid, str) or not uuid:
                raise ValueError("MP-10 missing index generation")
            return uuid

        before = generation()
        health = request_json("/_cluster/health/" + index)
        if health.get("status") not in ("green", "yellow") or health.get("timed_out") is not False:
            raise ValueError("MP-10 search health unavailable")
        total = request_json(f"/{index}/_count", query)
        # The frozen fixture's content renderer can label a shop page as a
        # product. Bind completeness to actual published product IDs instead
        # of assuming every document carrying that label is a catalog item.
        bound_query = {"query": {"bool": {"filter": [*query["query"]["bool"]["filter"], {"ids": {"values": ids}}]}}}
        count = request_json(f"/{index}/_count", bound_query)
        if total.get("_shards", {}).get("failed") != 0:
            raise ValueError("MP-10 search shards incomplete")
        if type(total.get("count")) is not int:
            raise ValueError("MP-10 search total count incomplete")
        if count.get("_shards", {}).get("failed") != 0:
            raise ValueError("MP-10 search shards incomplete")
        if type(count.get("count")) is not int:
            raise ValueError("MP-10 indexed product count unavailable")
        if count["count"] != expected:
            raise ValueError(f"MP-10 indexed product count mismatch: shop {shop['shop']}, expected {expected}, observed {count['count']}")
        if total["count"] < count["count"]:
            raise ValueError("MP-10 inconsistent search counts")
        if generation() != before:
            raise ValueError("MP-10 index generation changed during probe")
        observed.append({**{key: value for key, value in shop.items() if key != "publishedProductIds"},
                         "indexUuid": before, "indexedProducts": count["count"],
                         "otherProductDocuments": total["count"] - count["count"], "searchHealth": health["status"]})
    if time.monotonic() - started >= 30:
        raise ValueError("MP-10 search readiness deadline")
    return {"mpItems": ["MP-08", "MP-10"], "status": "ready", "observedAt": time.time(), "shops": observed}
