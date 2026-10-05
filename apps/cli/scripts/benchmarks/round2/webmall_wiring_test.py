"""MP-08/MP-10: admission checks ElasticPress's effective host, including filters."""
import ast, json, os, re, tempfile, unittest
from pathlib import Path
from webmall_readiness import probe_search

class SearchWiring(unittest.TestCase):
    def test_plugin_effective_host_overrides_stale_stored_option(self):
        source = Path(os.environ.get("WEBMALL_FIXTURE_SOURCE", str(Path(__file__).resolve().parent.parent / "webmall-sites.py")))
        tree = ast.parse(source.read_text())
        functions = [n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name in ("collect_search_readiness", "parse_search_metadata")]
        calls = []
        def docker(*args):
            calls.append(args)
            shop = int(args[1][-1])
            if "MP10_PUBLIC_SEARCH=" in args[-1]:
                data = {"host": "http://elasticsearch:9200/", "index": f"shop-{shop}-post-1", "publishedProducts": 1100}
                return "Notice: harmless fixture warning\nMP10_PUBLIC_SEARCH=" + json.dumps(data) + "\n", 0
            if "ep_host" in args: return "https://old-public-fixture.invalid", 0
            if "eval" in args: return f"shop-{shop}-post-1", 0
            return "1100", 0
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            (root / "catalog-restored.json").write_text(json.dumps([{"publishedProducts": 1100}] * 4))
            (root / "ports.json").write_text(json.dumps({"elasticsearch": 9200}))
            namespace = dict(json=json, re=re, docker=docker, evidence=root, root=root, PREFIX="r2next-webmall-r3",
                             probe_search=lambda endpoint, shops: {"status": "ready", "shops": shops})
            exec(compile(ast.Module(body=functions, type_ignores=[]), str(source), "exec"), namespace)
            result = namespace["collect_search_readiness"]()
            self.assertEqual(result["status"], "ready")
            self.assertEqual(len(result["shops"]), 4)
            self.assertTrue(all("get_host()" in args[-1] for args in calls))

if __name__ == "__main__": unittest.main()
