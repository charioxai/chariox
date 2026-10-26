"""MP-08/MP-10/MP-11: shared Room desktop bus admission and child environment."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

HELPER = Path(__file__).with_name('room-desktop-environment.py')

class RoomDesktopEnvironmentTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='chariox-room-desktop-env-')
        self.root = Path(self.temp.name)
        self.runtime = self.root / 'runtime'
        self.runtime.mkdir(mode=0o700)
        self.address = self.runtime / 'desktop-session-address'
        self.address.write_text('unix:abstract=chariox-public-test-bus')
        self.address.chmod(0o600)
        self.env = {**os.environ, 'CHARIOX_SLICE_PRIVATE_ROOT': str(self.root), 'DBUS_SESSION_BUS_ADDRESS': 'unix:abstract=stale-test-bus', 'NO_AT_BRIDGE': '1'}

    def tearDown(self):
        self.temp.cleanup()

    def launch(self):
        code = 'import os,json;print(json.dumps({k:os.environ.get(k) for k in ["DBUS_SESSION_BUS_ADDRESS","NO_AT_BRIDGE","GTK_A11Y","ACCESSIBILITY_ENABLED"]}))'
        return subprocess.run([sys.executable, str(HELPER), sys.executable, '-c', code], env=self.env, capture_output=True, text=True, timeout=8)

    def test_child_uses_current_private_desktop_bus_and_native_bridge(self):
        result = self.launch()
        self.assertEqual(result.returncode, 0)
        self.assertEqual(json.loads(result.stdout), {'DBUS_SESSION_BUS_ADDRESS': 'unix:abstract=chariox-public-test-bus', 'NO_AT_BRIDGE': '0', 'GTK_A11Y': 'always', 'ACCESSIBILITY_ENABLED': '1'})

    def test_unsafe_binding_never_launches_child(self):
        for contents, mode in [('unix:abstract=chariox-public-test-bus', 0o644), ('unix:abstract=bad bus', 0o600), ('tcp:host=localhost', 0o600), ('x' * 513, 0o600)]:
            with self.subTest(mode=mode, length=len(contents)):
                self.address.write_text(contents)
                self.address.chmod(mode)
                result = self.launch()
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')

    def test_symlink_binding_never_launches_child(self):
        target = self.root / 'other-bus'
        self.address.rename(target)
        self.address.symlink_to(target)
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')

    def test_standard_slice_root_binding(self):
        self.runtime.rename(self.root / 'private')
        del self.env['CHARIOX_SLICE_PRIVATE_ROOT']
        self.env['CHARIOX_SLICE_ROOT'] = str(self.root)
        self.assertEqual(self.launch().returncode, 0)

    def test_fifo_binding_is_refused_without_waiting_for_a_writer(self):
        self.address.unlink()
        os.mkfifo(self.address, 0o600)
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')

    @unittest.skipUnless(os.getuid() == 0, 'changing a test file owner requires root')
    def test_foreign_owner_never_launches_child(self):
        os.chown(self.address, 65534, 65534)
        result = self.launch()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '')

if __name__ == '__main__':
    unittest.main()
