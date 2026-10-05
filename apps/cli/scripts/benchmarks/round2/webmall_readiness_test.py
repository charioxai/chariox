"""MP-08 / MP-10 H10 fail-first: HTML/catalog readiness hides index failure."""
import copy
import unittest
from webmall_readiness import probe_search


class SearchReadiness(unittest.TestCase):
    def setUp(self):
        counts = [1150, 1095, 1156, 1020]  # published fixture metadata
        self.shops = [{"shop": i + 1, "index": f"shop-{i+1}-post-1",
                       "expectedProducts": n, "publishedProducts": n,
                       "publishedProductIds": [str(j + 1) for j in range(n)]} for i, n in enumerate(counts)]
        self.health = "yellow"  # a single-node replica need not be assigned
        self.failed_shards = 0
        self.count_delta = 0
        self.uuid_change = False
        self.calls = []
        self.non_catalog_product_documents = 0

    def request(self, path, body=None):
        self.calls.append((path, body))
        if path.startswith("/_cluster/health/"):
            return {"status": self.health, "timed_out": False}
        index = path.split("/")[1]
        if "_settings" in path:
            serial = sum(index + "/_settings" in call[0] for call in self.calls)
            return {index: {"settings": {"index": {"uuid": index + ("-new" if self.uuid_change and serial > 1 else "-old")}}}}
        count = next(s["expectedProducts"] for s in self.shops if s["index"] == index)
        filters = body["query"]["bool"]["filter"]
        self.assertEqual(filters[:2], [{"term": {"post_type": "product"}}, {"term": {"post_status": "publish"}}])
        extra = self.non_catalog_product_documents if not any("ids" in item for item in filters) else 0
        return {"count": count + self.count_delta + extra, "_shards": {"failed": self.failed_shards}}

    def test_public_catalog_ids_distinguish_misclassified_non_product_documents(self):
        self.non_catalog_product_documents = 1
        result = self.probe()
        self.assertEqual(result["status"], "ready")
        self.assertTrue(all(shop["indexedProducts"] == shop["expectedProducts"] for shop in result["shops"]))
        self.assertTrue(all(shop["otherProductDocuments"] == 1 for shop in result["shops"]))

    def probe(self, shops=None):
        return probe_search("http://127.0.0.1:9200", shops or self.shops, self.request)

    def test_catalog_and_html_do_not_admit_red_search(self):
        self.health = "red"
        with self.assertRaisesRegex(ValueError, "search health"):
            self.probe()

    def test_current_catalog_does_not_admit_partial_index(self):
        self.count_delta = -1
        with self.assertRaisesRegex(ValueError, "indexed product count"):
            self.probe()

    def test_count_from_failed_shards_is_not_complete(self):
        self.failed_shards = 1
        with self.assertRaisesRegex(ValueError, "search shards"):
            self.probe()

    def test_index_recreation_during_probe_is_not_ready(self):
        self.uuid_change = True
        with self.assertRaisesRegex(ValueError, "index generation"):
            self.probe()

    def test_all_four_fresh_indexes_are_bound_to_receipt(self):
        result = self.probe()
        self.assertEqual(result["status"], "ready")
        self.assertEqual(len(result["shops"]), 4)
        self.assertTrue(all(s.get("indexUuid") for s in result["shops"]))
        self.assertGreater(result["observedAt"], 0)

    def test_duplicate_index_does_not_certify_two_shops(self):
        shops = copy.deepcopy(self.shops)
        shops[1]["index"] = shops[0]["index"]
        with self.assertRaisesRegex(ValueError, "distinct indexes"):
            self.probe(shops)

    def test_missing_shop_remains_unadmitted(self):
        with self.assertRaisesRegex(ValueError, "all four shops"):
            self.probe(self.shops[:3])


if __name__ == "__main__":
    unittest.main()
