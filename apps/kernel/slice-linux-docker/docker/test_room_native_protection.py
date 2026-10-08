"""MP-08 / MP-11: Room helpers bind the private bus and reuse native admission."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('room_protection', Path(__file__).with_name('room-native-protection.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RoomProtectionTests(unittest.TestCase):
    def test_root_or_missing_slice_cannot_admit_a_login_desktop(self):
        for uid, env in [(0, {'CHARIOX_SLICE_ID': 'slice'}), (1001, {})]:
            with patch.object(module.os, 'getuid', return_value=uid), patch.dict(module.os.environ, env, clear=True), patch.object(module.os, 'open') as opening:
                with self.assertRaises(ValueError): module.binding()
                opening.assert_not_called()

    def test_wrong_owner_or_public_bus_binding_is_rejected_and_closed(self):
        for owner, mode in [(42, 0o100600), (1001, 0o100644)]:
            with patch.object(module.os, 'getuid', return_value=1001), patch.dict(module.os.environ, {'CHARIOX_SLICE_ID':'slice'}, clear=True), \
                 patch.object(module.os, 'open', return_value=99), patch.object(module.os, 'fstat', return_value=SimpleNamespace(st_mode=mode, st_uid=owner, st_size=10)), \
                 patch.object(module.os, 'read') as read, patch.object(module.os, 'close') as close:
                with self.assertRaises(ValueError): module.binding()
                read.assert_not_called()
                close.assert_called_once_with(99)

    def test_room_input_uses_the_same_native_guard_and_converts_unproved_binding_to_refusal(self):
        processes = [{'pid':42, 'started':'100'}]
        guard = Mock()
        accessibility = SimpleNamespace(input_guard=Mock(return_value=guard))
        with patch.object(module, 'binding', return_value=processes), patch.object(module, 'accessibility', return_value=accessibility):
            self.assertIs(module.input_guard(), guard)
            accessibility.input_guard.assert_called_once_with(processes)
        with patch.object(module, 'binding', side_effect=ValueError('private bus unavailable')):
            with self.assertRaises(module.RoomInputDenied): module.input_guard()

    def test_room_capture_uses_the_exact_native_snapshot_masks(self):
        processes = [{'pid':42, 'started':'100'}]
        tree = {'available':True, 'complete':True, 'protected':False, 'masks':[[20,30,40,50]]}
        accessibility = SimpleNamespace(snapshot=Mock(return_value=tree))
        with patch.object(module, 'binding', return_value=processes), patch.object(module, 'accessibility', return_value=accessibility):
            self.assertIs(module.snapshot(), tree)
            accessibility.snapshot.assert_called_once_with(processes)


if __name__ == '__main__': unittest.main()
