#!/usr/bin/env python3
"""MP-08/MP-11: regression for the real drill's SIGKILL timing claims."""
import ast
import os
from pathlib import Path
from types import SimpleNamespace
import unittest


SOURCE = Path(os.environ.get('LAUNCHER_DRILL_SOURCE', Path(__file__).with_name('live-launcher-lifecycle-drill.py')))


def sigkill_rows():
    # Execute the actual drill statements with controlled time. A slow
    # replacement startup must never extend the old child's settlement budget.
    tree = ast.parse(SOURCE.read_text())
    body = next(node for node in ast.walk(tree) if isinstance(node, ast.Try)
                and any(isinstance(child, ast.Constant) and child.value == 'sigkill-old'
                        for child in ast.walk(node))).body
    start_index = next(i for i, node in enumerate(body) if any(
        isinstance(child, ast.Constant) and child.value == 'sigkill-old' for child in ast.walk(node)))
    end_index = next(i for i, node in enumerate(body[start_index:], start_index) if any(
        isinstance(child, ast.Constant) and child.value == 'normal-quit' for child in ast.walk(node))) - 1
    clock = SimpleNamespace(now=100.0, killed=None)
    events = []
    rows = []
    def start(label, path):
        events.append(label)
        if label == 'sigkill-replacement':
            clock.now += 5.0
        return SimpleNamespace(pid=10, poll=lambda: 0), 20 if label == 'sigkill-old' else 30
    def send(*args):
        clock.killed = clock.now
        events.append('SIGKILL')
    def alive(pid):
        return pid == 20 and clock.now < clock.killed + .3
    def wait_for(predicate, timeout):
        deadline = clock.now + timeout
        while clock.now < deadline:
            if predicate():
                return True
            clock.now += .025
        return False
    def check(name, ok, **details):
        events.append(name)
        rows.append(dict(scenario=name, ok=ok, **details))
    namespace = dict(start=start, path=None, send=send, signal=SimpleNamespace(SIGKILL=9),
                     owned=None, pid_signals=None, wait_for=wait_for, alive=alive,
                     check=check, ready=lambda path: True, terminals=[(0, ''), (0, '')],
                     os=SimpleNamespace(write=lambda *args: None),
                     time=SimpleNamespace(monotonic=lambda: clock.now, sleep=lambda delay: None))
    exec(compile(ast.Module(body=body[start_index:end_index], type_ignores=[]), str(SOURCE), 'exec'), namespace)
    return rows, events


class SigkillTimingTests(unittest.TestCase):
    def test_parent_settlement_is_timed_from_kill_before_replacement(self):
        rows, events = sigkill_rows()
        row = next(row for row in rows if row['scenario'] == 'launcher-SIGKILL-parent-death')
        self.assertTrue(row['ok'])
        self.assertGreaterEqual(row['elapsedMs'], 300)
        self.assertLess(row['elapsedMs'], 2000)
        self.assertLess(events.index('launcher-SIGKILL-parent-death'), events.index('sigkill-replacement'))

    def test_replacement_row_claims_only_post_settlement_availability(self):
        rows, _ = sigkill_rows()
        self.assertIn('replacement-socket-after-settled-old-exit', [row['scenario'] for row in rows])
        self.assertNotIn('replacement-socket-survives-old-exit', [row['scenario'] for row in rows])


if __name__ == '__main__':
    unittest.main()
