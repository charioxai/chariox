"""MP-08/MP-10/MP-11 warm Unicode keeps the physical mapping settle pace."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('native_computer_text', Path(__file__).with_name('native-computer.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class WarmTextPacing(unittest.TestCase):
    def test_ascii_keeps_the_immediate_owned_channel(self):
        connection = object()
        with patch.object(native.keyboard, 'type_text') as typed:
            native.input_action({'kind': 'text', 'text': 'public text'}, connection=connection)
        typed.assert_called_once_with('public text', pace_seconds=0, connection=connection)

    def test_unicode_keeps_the_existing_mapping_delivery_pace(self):
        # The real Chrome regression lost the last emoji of one 128-character
        # block and the first symbols of its successor when mappings recycled.
        connection = object()
        for value in ['x' * 127 + '🌍', '漢字🧪' * 40, 'Café']:
            with self.subTest(value=value), patch.object(native.keyboard, 'type_text') as typed:
                native.input_action({'kind': 'text', 'text': value}, connection=connection)
                typed.assert_called_once_with(value, pace_seconds=0.04, connection=connection)

    def test_one_shot_text_retains_the_admission_pacing_path(self):
        with patch.object(native.keyboard, 'type_text') as typed:
            native.input_action({'kind': 'text', 'text': '世界'})
        typed.assert_called_once_with('世界', before_press=None)


if __name__ == '__main__':
    unittest.main()
