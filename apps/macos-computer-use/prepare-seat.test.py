import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location('prepare', Path(__file__).with_name('prepare-seat.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
class ScopeTests(unittest.TestCase):
    def test_scope_uses_independently_bound_geometry_and_distinct_revoke_point(self):
        output = ('fixturePID=42 windowID=7\n'
            'probe=line5-before-r clickX=20 clickY=30 expectedLocation=125 referenceLocation=125\n'
            'probe=line5-after-r clickX=21 clickY=30 expectedLocation=126 referenceLocation=126\n'
            'probe=line1 clickX=2 clickY=3 expectedLocation=3 referenceLocation=3\n'
            'probe=line9 clickX=4 clickY=5 expectedLocation=233 referenceLocation=233\n'
            'fixture geometry scale=2\n')
        state = {'frontmostPID': 42, 'windows': [{'pid': 42, 'windowID': 7,
            'bounds': {'X': 0, 'Y': 0, 'Width': 100, 'Height': 100}}]}
        scope = m.make_scope(output, state)
        self.assertEqual(scope['point'], {'x': 21.0, 'y': 30.0})
        self.assertEqual(scope['revoke_location'], 125)
        self.assertEqual(scope['scale'], 2)
        state['frontmostPID'] = 99
        with self.assertRaises(RuntimeError): m.make_scope(output, state)
if __name__ == '__main__': unittest.main()
