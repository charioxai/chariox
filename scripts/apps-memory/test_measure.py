# MP-08 / MP-10: accounting and MP-11 signal guards, no real browser required.
import unittest
from unittest.mock import patch
from measure import quantile, summarize, stop_owned

class Accounting(unittest.TestCase):
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

if __name__=='__main__': unittest.main()
