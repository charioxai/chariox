# MP-08 / MP-10: accounting and MP-11 signal guards, no real browser required.
import json
import os
import pathlib
import subprocess
import tempfile
import unittest
from unittest.mock import patch
from measure import quantile, summarize, stop_owned, process_tree, resource, slice_labels, owned_slice_resource, category, validate_drill

class DrillEvidence(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix='appsbudget-evidence-')
        self.addCleanup(self.directory.cleanup)
        self.root = pathlib.Path(self.directory.name)
        self.validation = {'mp_items':['MP-08','MP-10'], 'calls_per_view':[2,3,4,5],
                           'distinct_installations':4, 'sessionless':True, 'app_mirror_denied':True}
        self.phase = {'topology':'host', 'views':0, 'activity':'finished', 'at_ms':200000}
        self.samples = [
            {'topology':'host', 'views':views, 'activity':activity, 'at_ms':100000 + index*30000,
             'sample_at':100 + index*30 + second, 'processes':[
                 {'class':klass,'rss_kib':1024,'pss_kib':512} for klass in ['browser','renderers']]}
            for index,(views,activity) in enumerate([(0,'idle'),(4,'idle'),(4,'interacting')])
            for second in range(20)]

    def check(self, topology='host'):
        (self.root/'validation.json').write_text(json.dumps(self.validation))
        (self.root/'phase.json').write_text(json.dumps(self.phase))
        validate_drill(self.root, topology, self.samples)

    def test_complete_host_and_slice_evidence(self):
        self.check()
        self.validation.update(sessionless=False, app_mirror_denied=False)
        self.phase['topology'] = 'slice'
        for sample in self.samples: sample['topology'] = 'slice'
        self.check('slice')

    def test_incomplete_validation_is_red(self):
        original = dict(self.validation)
        for field, value in [('mp_items', []), ('calls_per_view', []), ('calls_per_view', [2,2,2,1]),
                             ('calls_per_view', [2,2,2,True]), ('distinct_installations', 3),
                             ('sessionless', False), ('app_mirror_denied', False)]:
            with self.subTest(field=field, value=value):
                self.validation = {**original, field:value}
                with self.assertRaises(RuntimeError): self.check()

    def test_missing_malformed_or_nonobject_evidence_is_red(self):
        self.check()
        for name in ['validation.json','phase.json']:
            for value in [None, '{', '[]']:
                with self.subTest(name=name, value=value):
                    self.check()
                    path = self.root/name
                    if value is None: path.unlink()
                    else: path.write_text(value)
                    with self.assertRaises(RuntimeError): validate_drill(self.root,'host',self.samples)

    def test_unfinished_wrong_topology_or_early_phase_is_red(self):
        for field,value in [('activity','cleanup'), ('topology','slice'), ('views',4), ('at_ms',110000)]:
            with self.subTest(field=field):
                original = dict(self.phase)
                self.phase[field] = value
                with self.assertRaises(RuntimeError): self.check()
                self.phase = original

    def test_each_baseline_and_four_view_window_is_required(self):
        for views,activity in [(0,'idle'),(4,'idle'),(4,'interacting')]:
            with self.subTest(views=views, activity=activity):
                samples = self.samples
                self.samples = [s for s in samples if (s['views'],s['activity']) != (views,activity)]
                with self.assertRaises(RuntimeError): self.check()
                self.samples = samples

    def test_empty_censored_short_or_outside_windows_are_red(self):
        for fault in ['empty','missing_rss','missing_pss','short','outside','no_marker','too_few']:
            with self.subTest(fault=fault):
                samples = self.samples
                self.samples = json.loads(json.dumps(samples))
                for sample in self.samples:
                    if fault == 'empty': sample['processes'] = []
                    if fault == 'missing_rss': sample['processes'][0]['rss_kib'] = None
                    if fault == 'missing_pss': sample['processes'][0]['pss_kib'] = None
                    if fault == 'short': sample['sample_at'] = sample['at_ms']/1000 + .1
                    if fault == 'outside': sample['sample_at'] += 20
                    if fault == 'no_marker': sample.pop('at_ms')
                if fault == 'too_few': self.samples = self.samples[::19]
                with self.assertRaises(RuntimeError): self.check()
                self.samples = samples

@unittest.skipUnless(os.environ.get('CHARIOX_APPS_MEMORY_WRAPPER_TEST') == '1',
                     'MP-08 / MP-10: set CHARIOX_APPS_MEMORY_WRAPPER_TEST=1 on the root measurement builder')
class WrapperIntegration(unittest.TestCase):
    def test_zero_exit_without_drill_evidence_is_red(self):
        # MP-08 / MP-10: exercise the real wrapper/child/cleanup path, without a browser.
        with tempfile.TemporaryDirectory(prefix='appsbudget-review-') as output:
            child = subprocess.run([
                'python3', str(pathlib.Path(__file__).with_name('measure.py')),
                '--binary', '/bin/true', '--output', output,
                '--topology', 'slice', '--image', 'unused-no-drill',
            ], capture_output=True, text=True)
            receipt = json.loads((pathlib.Path(output)/'receipt.json').read_text())
            self.assertEqual(receipt['exit_code'], 0)
            self.assertTrue(receipt['cleanup'])
            self.assertFalse(pathlib.Path(receipt['root']).exists())
            self.assertNotEqual(child.returncode, 0, child.stdout + child.stderr)
            self.assertEqual(receipt['result'], 'RED')

class Accounting(unittest.TestCase):
    def test_next_view_opening_is_not_charged_to_previous_interaction(self):
        samples = [{'topology':'host','views':2,'activity':'interacting','at_ms':100000,
                    'sample_at':at, 'processes':[{'class':'browser','rss_kib':rss,'pss_kib':rss}]}
                   for at,rss in [(119,1024),(121,4096)]]
        row = next(r for r in summarize(samples) if r['class']=='browser_total')
        self.assertEqual(row['rss_p95_mib'], 1)
        self.assertEqual(row['samples_after_window'], 1)

    def test_browser_total_includes_crashpad_helpers(self):
        self.assertEqual(category('/lane/chromium/chrome_crashpad_handler --database=/lane/profile'), 'utility')

    def test_repeated_slice_id_cannot_admit_another_kernel_resource(self):
        ownership = {'slice_id':'slice-1', 'owner_kernel_id':'kernel-own',
                     'runtime_name':'chariox-slice-appsbudget-own'}
        labels = slice_labels(ownership)
        self.assertTrue(owned_slice_resource(labels, ownership))
        for field in labels:
            foreign = dict(labels); foreign[field] = 'foreign'
            self.assertFalse(owned_slice_resource(foreign, ownership))
        self.assertFalse(owned_slice_resource({'io.chariox.slice.id':'slice-1'}, ownership))

    def test_cleanup_can_record_resources_below_the_floor(self):
        from types import SimpleNamespace
        with patch('measure.pathlib.Path.read_text', return_value='MemAvailable: 1024 kB\n'), \
             patch('measure.shutil.disk_usage', return_value=SimpleNamespace(free=1024)):
            with self.assertRaises(RuntimeError): resource()
            self.assertEqual(resource(check=False), {'mem_available_kib':1024,'disk_free_bytes':1024})

    def test_nearest_rank_and_simultaneous_class_totals(self):
        self.assertEqual(quantile([1024,2048,3072,4096], .95), 4)
        samples = [{'topology':'host','views':4,'activity':'idle','processes':[
            {'class':'browser','rss_kib':1024,'pss_kib':512},
            {'class':'renderers','rss_kib':2048,'pss_kib':256},
            {'class':'renderers','rss_kib':3072,'pss_kib':128},
            {'class':'GPU','rss_kib':1024,'pss_kib':512},
            {'class':'controller','rss_kib':999999,'pss_kib':999999},
        ]}]
        total = next(r for r in summarize(samples) if r['class']=='browser_total')
        self.assertEqual(total['rss_p95_mib'], 7)
        self.assertEqual(total['pss_p95_mib'], 1.375)

    def test_missing_pss_is_reported_not_silently_zeroed(self):
        result = summarize([{'topology':'host','views':1,'activity':'idle','processes':[
            {'class':'renderers','rss_kib':1024,'pss_kib':None}]}])
        renderer = next(r for r in result if r['class']=='renderers')
        self.assertEqual(renderer['pss_samples'], 0)
        self.assertIsNone(renderer['pss_p95_mib'])

    def test_signal_rejects_system_and_invalid_pids(self):
        for pid in [0,1,-1,-2,None,float('nan')]:
            with self.subTest(pid=pid), patch('measure.os.kill') as kill:
                with self.assertRaises(ValueError): stop_owned(pid,'identity',15)
                kill.assert_not_called()

    def test_reused_pid_is_never_signalled(self):
        with patch('measure.process',return_value={'start':'new'}), patch('measure.os.kill') as kill:
            stop_owned(22,'old',15)
            kill.assert_not_called()

    def test_reused_root_does_not_admit_foreign_descendants(self):
        entries = [type('Entry', (), {'name':str(pid)})() for pid in [22,23]]
        rows = {22:{'pid':22,'ppid':1,'start':'new','cmd':'foreign'},
                23:{'pid':23,'ppid':22,'start':'child','cmd':'foreign child'}}
        retained = {22:'old'}
        with patch('measure.pathlib.Path.iterdir', return_value=entries), patch('measure.process', side_effect=rows.get):
            self.assertEqual(process_tree(22,retained), [])
        self.assertEqual(retained,{22:'old'})

if __name__=='__main__': unittest.main()
