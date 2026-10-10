#!/usr/bin/env python3
"""Headless checks of the unattended caret verdict and GUI hold gate."""
import importlib.util
import json
import os
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
        root.mkdir(parents=True, exist_ok=True)
        self.evidence = Path(tempfile.mkdtemp(prefix='caret-verdict-unit-', dir=root))
        self.state = {'frontmostPID': 42, 'pointer': {'x': 366.0, 'y': 571.0}}

    def tearDown(self):
        # This test owns only these public fake-output files. No recursive cleanup.
        files = list(self.evidence.iterdir())
        assert all(p.is_file() and p.name in ['probe.json', 'probe-warp.json', 'fixture.out', 'GUI_HOLD', 'GUI_OK'] for p in files)
        for path in files:
            path.unlink()
        self.evidence.rmdir()

    def verdict(self, receipt, selected):
        with patch.object(drill, 'require_gui'), patch.object(drill, 'desktop', return_value=self.state), \
                patch.object(drill, 'launch_helper', return_value=receipt), \
                patch.object(drill, 'selection', return_value=selected), \
                patch.object(drill.time, 'monotonic', side_effect=[0, 2]):
            return drill.verify_caret(self.evidence, 'probe', [], 42, '366', '571', '125', False)[0]

    def test_nondefault_build_selects_exact_helper_and_fixture(self):
        with patch.dict(os.environ, {'CUMAC_BUILD_DIR': '/Users/miguel/.chariox/dev/cumac/build/r6'}):
            spec = importlib.util.spec_from_file_location('configured', Path(__file__).with_name('text-click-drill.py'))
            configured = importlib.util.module_from_spec(spec)
            spec.loader.exec_module(configured)
        self.assertEqual(configured.BUILD, Path('/Users/miguel/.chariox/dev/cumac/build/r6'))
        self.assertEqual(configured.HELPER, configured.BUILD / 'Chariox Computer Helper.app')

    def test_receipt_refuses_other_build_before_accepting_evidence(self):
        expected = '/Users/miguel/.chariox/dev/cumac/build/r6/Chariox Computer Helper.app/Contents/MacOS/helper'
        receipt = {'path': expected, 'identifier': 'ai.chariox.computer-helper',
                   'signatureValid': True, 'cdhash': 'a' * 40, 'parentPID': 1}
        drill.verify_launch_receipt(json.dumps(receipt), Path(expected), 'a' * 40)
        for field, value in [('path', expected.replace('/r6/', '/')), ('cdhash', 'b' * 40),
                             ('signatureValid', False), ('parentPID', 2)]:
            with self.assertRaises(RuntimeError):
                drill.verify_launch_receipt(json.dumps({**receipt, field: value}), Path(expected), 'a' * 40)

    def test_exact_point_index_and_latest_selection_pass(self):
        self.assertTrue(self.verdict('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=125 length=0; point=366.0,571.0', ('125', '0')))

    def test_owner_wrong_index_fails(self):
        self.assertFalse(self.verdict('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=84 length=0; point=366.0,571.0', ('84', '0')))

    def test_wrong_coordinate_receipt_fails_even_with_expected_selection(self):
        self.assertFalse(self.verdict('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=125 length=0; point=182.457,147.887', ('125', '0')))

    def test_refusal_fails(self):
        self.assertFalse(self.verdict('refused: target (no-range-at-point)', ('0', '0')))

    def pointer_verdict(self, pointer, cg_pointer=None, receipt_point='363.0,571.0', refuse=False):
        state = {'frontmostPID': 42, 'pointer': pointer,
                 'cgPointer': cg_pointer if cg_pointer is not None else pointer}
        receipt = ('dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange '
                   f'location=125 length=0; point={receipt_point}')
        with patch.object(drill, 'require_gui'), patch.object(drill, 'desktop', return_value=state), \
                patch.object(drill, 'warp_pointer'), patch.object(drill, 'launch_helper', return_value=receipt) as launch, \
                patch.object(drill, 'selection', return_value=('125', '0')):
            if refuse:
                with self.assertRaisesRegex(RuntimeError, 'pointer-warp-coordinate-mismatch'):
                    drill.verify_caret(self.evidence, 'probe', [], 42,
                                       '363.61328125', '571', '125', True)
                launch.assert_not_called()
                return
            result = drill.verify_caret(self.evidence, 'probe', [], 42,
                                        '363.61328125', '571', '125', True)[0]
            launch.assert_called_once_with(self.evidence, 'probe', ['--target', 'text', '--click-at-pointer', '--click'])
            return result

    def test_fractional_warp_uses_sampled_point_for_receipt(self):
        # Actual fail-first geometry on this Mac: CG warp drops fractional x.
        self.assertTrue(self.pointer_verdict({'x': 363.0, 'y': 571.0}))

    def test_pointer_receipt_must_match_sample_not_fractional_request(self):
        self.assertFalse(self.pointer_verdict({'x': 363.0, 'y': 571.0}, receipt_point='363.61328125,571.0'))

    def test_one_point_or_larger_warp_error_refuses_before_helper(self):
        self.pointer_verdict({'x': 363.61328125, 'y': 572.0}, refuse=True)

    def test_cg_and_appkit_disagreement_refuses_before_helper(self):
        self.pointer_verdict({'x': 363.0, 'y': 571.0}, {'x': 363.0, 'y': 411.0}, refuse=True)

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

    def test_both_runner_modes_opt_into_fixture_watchdog(self):
        self.assertEqual(drill.fixture_arguments('text-click'), ['--unattended', '--text-drill'])
        self.assertEqual(drill.fixture_arguments('fixture'), ['--unattended'])

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
