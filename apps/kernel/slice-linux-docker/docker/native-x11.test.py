"""MP-08 / MP-11: same display number never selects a foreign filesystem socket."""
import importlib.util
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import patch

spec=importlib.util.spec_from_file_location('native_x11',Path(__file__).with_name('native-x11.py'))
adapter=importlib.util.module_from_spec(spec);spec.loader.exec_module(adapter)


class Socket:
    def __init__(self):self.address=None;self.closed=False;self.inheritable=True
    def settimeout(self,value):pass
    def set_inheritable(self,value):self.inheritable=value
    def connect(self,address):self.address=address
    def close(self):self.closed=True


class OwnedDisplay(unittest.TestCase):
    def fixture(self):
        foreign=Socket()
        transport=SimpleNamespace(get_socket=lambda *args:foreign)
        display=SimpleNamespace(__package__='Xlib',Display=lambda:transport.get_socket(':1',None,'',1))
        return transport,display,foreign

    def test_owned_abstract_overrides_foreign_filesystem_and_restores_library(self):
        transport,display,foreign=self.fixture();original=transport.get_socket;private=Socket()
        with patch.dict('os.environ',{'CHARIOX_OWNED_VIRTUAL_DISPLAY':'1'}),patch.object(adapter.importlib,'import_module',return_value=transport),patch.object(adapter.socket,'socket',return_value=private):
            self.assertIs(adapter.open_display(display),private)
        self.assertEqual(private.address,'\0/tmp/.X11-unix/X1')
        self.assertFalse(private.inheritable)
        self.assertIs(transport.get_socket,original)
        self.assertIsNone(foreign.address)

    def test_missing_owned_abstract_fails_without_foreign_fallback(self):
        transport,display,foreign=self.fixture();original=transport.get_socket;private=Socket()
        private.connect=lambda address:(_ for _ in ()).throw(ConnectionRefusedError())
        with patch.dict('os.environ',{'CHARIOX_OWNED_VIRTUAL_DISPLAY':'1'}),patch.object(adapter.importlib,'import_module',return_value=transport),patch.object(adapter.socket,'socket',return_value=private):
            with self.assertRaises(ConnectionRefusedError):adapter.open_display(display)
        self.assertTrue(private.closed);self.assertIs(transport.get_socket,original)
        self.assertIsNone(foreign.address)

    def test_ordinary_slice_display_uses_existing_library(self):
        transport,display,foreign=self.fixture()
        with patch.dict('os.environ',{'CHARIOX_OWNED_VIRTUAL_DISPLAY':'0'}):
            self.assertIs(adapter.open_display(display),foreign)

    def test_remote_display_keeps_existing_transport(self):
        transport,display,foreign=self.fixture()
        display.Display=lambda:transport.get_socket('host:1','tcp','host',1)
        with patch.dict('os.environ',{'CHARIOX_OWNED_VIRTUAL_DISPLAY':'1'}),patch.object(adapter.importlib,'import_module',return_value=transport):
            self.assertIs(adapter.open_display(display),foreign)


if __name__=='__main__':unittest.main()
