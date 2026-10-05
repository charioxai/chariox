"""MP-08/MP-10/MP-11: actual fixture command refuses unsafe child identities."""
import ast, os, signal, subprocess, tempfile, time, unittest
from pathlib import Path
from owned_processes import OwnedProcessTree

class FixtureCleanup(unittest.TestCase):
    def test_command_never_signals_system_or_missing_pid(self):
        source = Path(os.environ.get("WEBMALL_FIXTURE_SOURCE", str(Path(__file__).resolve().parent.parent / "webmall-sites.py")))
        command_ast = next(node for node in ast.parse(source.read_text()).body if isinstance(node, ast.FunctionDef) and node.name == "command")
        for pid in (None, 0, 1, -1, -20, float("nan")):
            calls = []
            class Child:
                returncode = None
                def poll(self): return None
                def terminate(self): calls.append((self.pid, "SIGTERM"))
                def kill(self): calls.append((self.pid, "SIGKILL"))
                def wait(self, **kwargs): self.returncode = -15; return self.returncode
            child = Child(); child.pid = pid
            class Subprocess:
                TimeoutExpired = subprocess.TimeoutExpired
                @staticmethod
                def Popen(*args, **kwargs): return child
            def guard(): raise RuntimeError("MP-10 synthetic floor")
            with tempfile.TemporaryDirectory() as root:
                namespace = dict(subprocess=Subprocess, tempfile=tempfile, time=time, signal=signal,
                                 OwnedProcessTree=OwnedProcessTree, root=Path(root), evidence=Path(root),
                                 cleaning=False, guard=guard, MP=["MP-08", "MP-10"])
                exec(compile(ast.Module(body=[command_ast], type_ignores=[]), str(source), "exec"), namespace)
                with self.assertRaises((ValueError, RuntimeError)):
                    namespace["command"](["synthetic-no-spawn"])
            self.assertEqual(calls, [], f"MP-11 unsafe cleanup child {pid}")

if __name__ == "__main__": unittest.main()
