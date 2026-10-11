"""The real CLI must stop its suite after a fixture cell loses ownership."""
from contextlib import redirect_stdout
import importlib.util
import io
import os
from pathlib import Path
import runpy
import subprocess
import sys
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('fixture_suite_lifecycle', Path(__file__).with_name('fixture-lifecycle-fixture.py'))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class FixtureSuiteTests(lifecycle.FixtureLifecycleCase):
    def test_suite_stops_before_another_cell_after_restoration_retirement(self):
        self.fail_on = 1
        self.restore_failure = 'focus'
        script = Path(__file__).with_name('text-click-drill.py')
        with redirect_stdout(io.StringIO()), \
             patch.dict(os.environ, {'CUMAC_BUILD_DIR': str(lifecycle.drill.BUILD), 'CUMAC_GUI_WAIT_SECONDS': '0'}), \
             patch.object(sys, 'argv', [str(script), '--suite']), \
             patch.object(subprocess, 'run', side_effect=self.native), \
             patch.object(Path, 'home', return_value=self.root), \
             patch.object(lifecycle.drill.time, 'strftime', side_effect=['suite-text', 'suite-button']), \
             self.assertRaises(SystemExit) as exit_result:
            runpy.run_path(str(script), run_name='__main__')
        self.assertEqual(exit_result.exception.code, 1)
        self.assertTrue(self.restored_facts)
        self.assertEqual(len(self.launches), 8)
        self.assertEqual(self.pointer_clicks, ['line5-before-r-pointer'])
        self.assertEqual(self.warps, [{'x': 20.0, 'y': 30.0}])
        evidence = self.root / '.codex/evidence/browser-computer-use-macos'
        self.assertFalse((evidence / 'suite-button').exists())


if __name__ == '__main__': unittest.main()
