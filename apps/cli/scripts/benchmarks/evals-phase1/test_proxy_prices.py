"""MP-08 / MP-10: missing billing categories remain explicit in proxy bounds."""
import unittest
from proxy_prices import proxy_quote

class ProxyTests(unittest.TestCase):
    def test_missing_band_and_writes_produce_interval(self):
        q = proxy_quote('gpt-6.1-sol', dict(input_tokens=1000,cached_input_tokens=800,output_tokens=100,reasoning_tokens=50))
        self.assertEqual(q['unknown_fields'], ['context_band', 'cache_write_tokens'])
        self.assertEqual(q['lower_nanodollars'], 1480000)
        self.assertEqual(q['upper_nanodollars'], 3460000)
        self.assertIsNone(q['point_nanodollars'])

    def test_reasoning_is_subset_of_output(self):
        a = dict(input_tokens=1000,cached_input_tokens=800,output_tokens=100,reasoning_tokens=0)
        b = dict(a,reasoning_tokens=50)
        self.assertEqual(proxy_quote('gpt-6.1-sol',a),proxy_quote('gpt-6.1-sol',b))

    def test_unknown_model_and_bad_counts_fail_closed(self):
        self.assertIsNone(proxy_quote('unmapped', {}))
        for usage in [dict(input_tokens=1,cached_input_tokens=2,output_tokens=0),dict(input_tokens=-1,cached_input_tokens=0,output_tokens=0)]:
            with self.assertRaises(ValueError): proxy_quote('gpt-6.1-sol',usage)

    def test_known_categories_can_produce_labeled_point(self):
        q=proxy_quote('gpt-6.1-sol',dict(input_tokens=1000,cached_input_tokens=800,output_tokens=100,context_band='short',cache_write_tokens=100))
        self.assertEqual(q['point_nanodollars'],1730000)
        self.assertEqual(q['unknown_fields'],[])
        self.assertEqual(q['label'],'proxy')
