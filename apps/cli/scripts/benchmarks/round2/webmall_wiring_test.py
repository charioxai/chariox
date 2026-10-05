"""MP-08/MP-10: admission checks ElasticPress's effective host, including filters."""
import ast, json, os, re, tempfile, unittest
from pathlib import Path
from webmall_readiness import probe_search

class SearchWiring(unittest.TestCase):
    def reset_function(self):
        source = Path(os.environ.get("WEBMALL_FIXTURE_SOURCE", str(Path(__file__).resolve().parent.parent / "webmall-sites.py")))
        tree = ast.parse(source.read_text())
        found = next((n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == "sync_search_indexes"), None)
        if found: return source, found
        # Historical adapter's exact setup loop, before the reset was extracted.
        setup = next(n for n in tree.body if isinstance(n, ast.Try))
        loop = next(n for n in reversed(setup.body) if isinstance(n, ast.For))
        return source, ast.FunctionDef(name="sync_search_indexes", args=ast.arguments(posonlyargs=[], args=[], kwonlyargs=[], kw_defaults=[], defaults=[]), body=[loop], decorator_list=[])

    def test_complete_reset_precedes_each_shop_sync(self):
        source, function = self.reset_function(); calls = []
        def docker(*args, **kwargs):
            if args[0] == "top": return "PID PPID COMMAND\n100 1 httpd\n", 0
            if "eval" in args: return "public metadata", 0
            calls.append(args); return "", 0
        namespace = dict(PREFIX="r2next-webmall-r3", docker=docker,
                         parse_search_metadata=lambda output: {"host": "http://elasticsearch:9200"},
                         inspect=lambda *args: {"Config": {"Labels": {"io.chariox.benchmark.lane": "r2next-webmall-r3"}}})
        exec(compile(ast.fix_missing_locations(ast.Module(body=[function], type_ignores=[])), str(source), "exec"), namespace)
        namespace["sync_search_indexes"]()
        self.assertEqual(len(calls), 8)
        for offset in range(0, 8, 2):
            self.assertEqual(calls[offset][3:], ("wp", "elasticpress", "clear-sync"))
            self.assertEqual(calls[offset + 1][3:6], ("wp", "elasticpress", "sync"))
            self.assertIn("--setup", calls[offset + 1])
            self.assertEqual(calls[offset][2], calls[offset + 1][2])

    def test_unowned_shop_never_receives_index_reset(self):
        source, function = self.reset_function(); calls = []
        namespace = dict(PREFIX="r2next-webmall-r3", docker=lambda *args, **kwargs: calls.append(args),
                         inspect=lambda *args: {"Config": {"Labels": {"io.chariox.benchmark.lane": "other-lane"}}})
        exec(compile(ast.fix_missing_locations(ast.Module(body=[function], type_ignores=[])), str(source), "exec"), namespace)
        with self.assertRaisesRegex(AssertionError, "unowned"):
            namespace["sync_search_indexes"]()
        self.assertEqual(calls, [])

    def test_live_cli_worker_prevents_stale_metadata_clear(self):
        source, function = self.reset_function(); calls = []
        def docker(*args, **kwargs):
            calls.append(args)
            return "PID PPID COMMAND\n123 1 php\n", 0
        namespace = dict(PREFIX="r2next-webmall-r3", docker=docker,
                         inspect=lambda *args: {"Config": {"Labels": {"io.chariox.benchmark.lane": "r2next-webmall-r3"}}})
        exec(compile(ast.fix_missing_locations(ast.Module(body=[function], type_ignores=[])), str(source), "exec"), namespace)
        with self.assertRaisesRegex(AssertionError, "live CLI"):
            namespace["sync_search_indexes"]()
        self.assertTrue(all(args[0] == "top" for args in calls))

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
