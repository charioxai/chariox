#!/usr/bin/env python3
"""Headless regression checks for the real TextEdit receipt/selection verdict."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('textedit_drill', Path(__file__).with_name('textedit-click-drill.py'))
drill = importlib.util.module_from_spec(spec)
spec.loader.exec_module(drill)


class TextEditVerdictTests(unittest.TestCase):
    def test_manual_wrong_point_and_index_are_rejected(self):
        receipt = 'dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=84 length=0; point=182.45703125,147.88671875'
        self.assertFalse(drill.caret_matches(receipt, 126, {'x': 316, 'y': 192}))

    def test_receipt_readback_must_match_index_and_point(self):
        receipt = 'dispatched; path=AXSelectedTextRange; observed AXSelectedTextRange location=126 length=0; point=316.0,192.0'
        self.assertTrue(drill.caret_matches(receipt, 126, {'x': 316, 'y': 192}))
        self.assertFalse(drill.caret_matches(receipt, 125, {'x': 316, 'y': 192}))
        self.assertFalse(drill.caret_matches(receipt, 126, {'x': 315, 'y': 192}))

    def test_refusal_and_unproven_completion_fail(self):
        for receipt in ['refused: permission (accessibility-denied)', 'dispatched; path=AXSelectedTextRange; application completion unproven']:
            self.assertFalse(drill.caret_matches(receipt, 126, {'x': 316, 'y': 192}))


if __name__ == '__main__':
    unittest.main()
