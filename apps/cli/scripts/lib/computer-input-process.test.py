"""MP-08/MP-10/MP-11: forbidden IDs never reach signal syscalls."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("owned", Path(__file__).with_name("computer-input-process.py"))
owned = importlib.util.module_from_spec(spec)
spec.loader.exec_module(owned)


class OwnedSignals(unittest.TestCase):
    def setUp(self):
        self.tree = {10: {"parent": 2, "group": 10, "start": "100"},
                     11: {"parent": 10, "group": 10, "start": "101"}}
        self.sent = []

    def group(self, pid=10, start="100"):
        return owned.signal_owned_group({"pid": pid, "start": start}, 15,
            read_stat=self.tree.__getitem__, pids=list(self.tree), send=lambda *args: self.sent.append(args))

    def test_forbidden_ids(self):
        for pid in [0, 1, -1, None, float("nan"), "10", True]:
            with self.subTest(pid=pid), self.assertRaises(ValueError):
                self.group(pid)
        self.assertEqual(self.sent, [])

    def test_reused_group(self):
        with self.assertRaises(ValueError):
            self.group(start="99")
        self.assertEqual(self.sent, [])

    def test_foreign_member(self):
        self.tree[11]["parent"] = 2
        with self.assertRaises(ValueError):
            self.group()
        self.assertEqual(self.sent, [])

    def test_own_group(self):
        self.group()
        self.assertEqual(self.sent, [(10, 15)])

    def test_child_ids_and_ownership(self):
        parent = {"pid": 10, "start": "100"}
        for pid in [0, 1, -1, None, float("nan")]:
            with self.subTest(pid=pid), self.assertRaises(ValueError):
                owned.signal_owned_child({"pid": pid, "start": "101"}, parent, 3, send=lambda *a: self.sent.append(a))
        for start in ["99", "101"]:
            self.tree[11]["parent"] = 2
            with self.assertRaises(ValueError):
                owned.signal_owned_child({"pid": 11, "start": start}, parent, 3,
                    read_stat=self.tree.__getitem__, send=lambda *a: self.sent.append(a))
        self.assertEqual(self.sent, [])
        self.tree[11]["parent"] = 10
        owned.signal_owned_child({"pid": 11, "start": "101"}, parent, 3,
            read_stat=self.tree.__getitem__, send=lambda *a: self.sent.append(a))
        self.assertEqual(self.sent, [(11, 3)])


if __name__ == "__main__":
    unittest.main()
