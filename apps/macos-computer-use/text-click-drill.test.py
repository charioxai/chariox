#!/usr/bin/env python3
"""Headless checks of the unattended caret verdict and GUI hold gate."""
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('drill', Path(__file__).with_name('text-click-drill.py'))
drill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drill)


class CaretVerdictTests(unittest.TestCase):
    def setUp(self):
        root = Path.home() / '.codex/evidence/browser-computer-use-macos'
        self.evidence = Path(tempfile.mkdtemp(prefix='caret-verdict-unit-', dir=root))
        self.state = {'frontmostPID': 42, 'pointer': {'x': 366.0, 'y': 571.0}}

    def tearDown(self):
        # This test owns only these public fake-output files. No recursive cleanup.
        files = list(self.evidence.iterdir())
        assert all(p.is_file() and p.name in ['probe.json', 'fixture.out', 'GUI_HOLD', 'GUI_OK'] for p in files)
        for path in files:
            path.unlink()
        self.evidence.rmdir()

    def verdict(self, receipt, selected):
        with patch.object(drill, 'require_gui'), patch.object(drill, 'desktop', return_value=self.state), \
                patch.object(drill, 'launch_helper', return_value=receipt), \
                patch.object(drill, 'selection', return_value=selected), \
                patch.object(drill.time, 'monotonic', side_effect=[0, 2]):
            return drill.verify_caret(self.evidence, 'probe', [], 42, '366', '571', '125', False)[0]

    def test_exact_point_index_and_latest_selection_pass(self):
        self.assertTrue(self.verdict('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=125 length=0; point=366.0,571.0', ('125', '0')))

    def test_owner_wrong_index_fails(self):
        self.assertFalse(self.verdict('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=84 length=0; point=366.0,571.0', ('84', '0')))

    def test_wrong_coordinate_receipt_fails_even_with_expected_selection(self):
        self.assertFalse(self.verdict('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=125 length=0; point=182.457,147.887', ('125', '0')))

    def test_refusal_fails(self):
        self.assertFalse(self.verdict('refused: target (no-range-at-point)', ('0', '0')))

    def test_selection_reads_last_observation(self):
        (self.evidence / 'fixture.out').write_text('fixture selection location=125 length=0\nfixture selection location=84 length=0\n')
        self.assertEqual(drill.selection(self.evidence), ('84', '0'))

    def test_lost_focus_never_launches_helper(self):
        with patch.object(drill, 'require_gui'), patch.object(drill, 'desktop', return_value={'frontmostPID': 43}), \
                patch.object(drill, 'launch_helper') as launch:
            with self.assertRaisesRegex(RuntimeError, 'fixture-lost-frontmost'):
                drill.verify_caret(self.evidence, 'probe', [], 42, '366', '571', '125', False)
            launch.assert_not_called()

    def test_all_four_independent_fixture_probes_are_required(self):
        output = '\n'.join(f'probe={name} clickX=366 clickY=571 expectedLocation={index} referenceLocation={index}'
                           for name, index in [('line5-before-r',125), ('line5-after-r',126), ('line1',3), ('line9',233)])
        self.assertEqual(len(drill.fixture_probes(output)), 4)
        with self.assertRaisesRegex(RuntimeError, 'fixture-independent-reference-mismatch'):
            drill.fixture_probes(output.splitlines()[0])

    def test_bad_fixture_reference_fails_before_dispatch(self):
        output = '\n'.join(f'probe={name} clickX=366 clickY=571 expectedLocation={index} referenceLocation={index}'
                           for name, index in [('line5-before-r',125), ('line5-after-r',126), ('line1',3), ('line9',233)])
        for bad in [output.replace('referenceLocation=125', 'referenceLocation=84'),
                    output.replace('expectedLocation=125 referenceLocation=125', 'expectedLocation=84 referenceLocation=84')]:
            with self.assertRaisesRegex(RuntimeError, 'fixture-independent-reference-mismatch'):
                drill.fixture_probes(bad)

    def test_hold_release_override_and_twenty_minute_timeout(self):
        with patch.object(drill, 'LANE', self.evidence), patch.object(drill, 'GUI_DEADLINE', 1200), \
                patch.object(drill.time, 'monotonic', return_value=0):
            self.assertTrue(drill.gui_allowed())
            (self.evidence / 'GUI_HOLD').touch()
            self.assertFalse(drill.gui_allowed())
            (self.evidence / 'GUI_OK').touch()
            self.assertTrue(drill.gui_allowed())
            (self.evidence / 'GUI_OK').unlink()
            with patch.object(drill.time, 'monotonic', return_value=1200):
                self.assertTrue(drill.gui_allowed())


if __name__ == '__main__':
    unittest.main()
