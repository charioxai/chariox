// MP-11: Python guard joins the same CI Node entry; no destructive invalid signal is sent.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { test } from 'node:test'
import { fileURLToPath } from 'node:url'

test('MP-11 Python invalid, foreign, reused and live owned-group regressions', () => {
  const root = fileURLToPath(new URL('../apps/kernel/slice-linux-docker/', import.meta.url))
  const output = execFileSync('python3', ['-B', '-c', `
import sys,unittest,subprocess,signal
sys.path.insert(0,sys.argv[1])
from owned_process_signals import OwnedProcesses
class GuardTests(unittest.TestCase):
    def fixture(self):
        rows=[dict(pid=50,ppid=9,pgid=50,start='old')]
        sent=[]
        guard=OwnedProcesses(snapshot=lambda:rows,send_group=lambda *args:sent.append(args),send_pid=lambda *args:sent.append(args))
        guard.record(50)
        return rows,sent,guard
    def test_invalid(self):
        for pid in [0,1,-1,-2,None,float('nan'),float('inf'),1.5,'50',True]:
            rows,sent,g=self.fixture()
            for method in [g.record,lambda pid:g.group(pid,signal.SIGTERM),lambda pid:g.pid(pid,signal.SIGTERM)]:
                with self.assertRaises(ValueError):method(pid)
            self.assertEqual(sent,[])
    def test_foreign(self):
        rows,sent,g=self.fixture();rows.append(dict(pid=90,ppid=9,pgid=50,start='foreign'))
        with self.assertRaises(ValueError):g.group(50,signal.SIGTERM)
        self.assertEqual(sent,[])
    def test_reuse(self):
        rows,sent,g=self.fixture();rows[0]['start']='reused'
        with self.assertRaises(ValueError):g.group(50,signal.SIGTERM)
        self.assertEqual(sent,[])
    def test_exit(self):
        rows,sent,g=self.fixture();rows.append(dict(pid=51,ppid=50,pgid=50,start='child'));g.refresh()
        rows[:]=[dict(pid=51,ppid=1,pgid=50,start='child')];self.assertTrue(g.group(50,signal.SIGKILL))
        rows[0]['start']='reused'
        with self.assertRaises(ValueError):g.group(50,signal.SIGKILL)
        self.assertEqual(len(sent),1)
    def test_live(self):
        child=subprocess.Popen([sys.executable,'-c','import time;time.sleep(10)'],start_new_session=True)
        g=OwnedProcesses();g.record(child.pid)
        try:self.assertTrue(g.group(child.pid,signal.SIGTERM));child.wait(timeout=3)
        finally:
            if child.poll() is None:g.group(child.pid,signal.SIGKILL);child.wait(timeout=3)
            g.close()
        self.assertFalse(g.group(child.pid,signal.SIGKILL))
unittest.main(argv=['guard-tests'],verbosity=2)
`, root], { encoding: 'utf8', timeout: 10_000, stdio: ['ignore', 'pipe', 'pipe'] })
  assert.equal(output, '')
})
