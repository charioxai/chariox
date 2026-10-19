"""MP-08/MP-11: ordinary input proves recipients without reading secret fields."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('native_computer', Path(__file__).with_name('native-computer.py'))
native = importlib.util.module_from_spec(spec)
spec.loader.exec_module(native)


class OrdinaryInput(unittest.TestCase):
    def test_chromium_and_unrelated_native_keys_do_not_walk_secret_fields(self):
        for browser in (True, False):
            for action in ({'kind': 'text', 'text': 'authorized'}, {'kind': 'key', 'key': 'Enter'}):
                with self.subTest(browser=browser, kind=action['kind']):
                    processes = [{'pid': 200, 'started': '1', 'browser': browser}]
                    accessibility = SimpleNamespace(input_guard=Mock(side_effect=AssertionError('input must not inspect field values')))
                    clipboard = SimpleNamespace(input_admission=Mock(return_value=lambda: None))
                    recipient = SimpleNamespace(input_destination=Mock(), key_destinations=Mock())
                    connection = SimpleNamespace(close=Mock())
                    x11 = SimpleNamespace(open_display=Mock(return_value=connection))
                    modules = {'native-accessibility': accessibility, 'native-clipboard': clipboard,
                               'native-input-destination': recipient, 'native-x11': x11}
                    def dispatch(*args, **kwargs):
                        kwargs['before_press']()
                    with patch.object(native, 'load', side_effect=modules.__getitem__), \
                         patch.object(native.keyboard, 'type_text', side_effect=dispatch), \
                         patch.object(native.keyboard, 'hold_input', side_effect=dispatch):
                        values=('private-test-canary',)
                        native.input_action(action, processes, values=values)
                    accessibility.input_guard.assert_not_called()
                    recipient.input_destination.assert_called_once_with(connection, processes, accessibility)
                    clipboard.input_admission.assert_called_once_with(processes, accessibility, None, values)
                    connection.close.assert_called_once()


if __name__ == '__main__': unittest.main()
