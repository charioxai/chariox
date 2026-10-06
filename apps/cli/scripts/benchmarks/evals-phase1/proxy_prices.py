"""MP-08 / MP-10: explicit dated proxy mapping; unknown categories stay bounded."""
import json
from pathlib import Path


def proxy_quote(model, usage, table=None):
    table = table or json.loads(Path(__file__).with_name('proxy-prices-2026-10-06.json').read_text())
    mapping = table['mappings'].get(model)
    if mapping is None:
        return None
    counts = [usage.get(k) for k in ['input_tokens','cached_input_tokens','output_tokens']]
    if any(type(n) is not int or n < 0 for n in counts) or counts[1] > counts[0]:
        raise ValueError('MP-08 / MP-10: invalid known proxy counts')
    inputs, cached, output = counts
    band, writes = usage.get('context_band'), usage.get('cache_write_tokens')
    if band is not None and band not in mapping['bands']:
        raise ValueError('MP-08 / MP-10: unknown supplied context band')
    if writes is not None and (type(writes) is not int or not 0 <= writes <= inputs-cached):
        raise ValueError('MP-08 / MP-10: cache-write count outside proxy bound')
    bands = [mapping['bands'][band]] if band is not None else list(mapping['bands'].values())
    def amount(price, write_count):
        # Reasoning is already included in output. Do not add it a second time.
        return (inputs-cached)*price['input'] + cached*price['cached_input'] + output*price['output'] + write_count*price['cache_write']
    lower = min(amount(p, writes if writes is not None else 0) for p in bands)
    upper = max(amount(p, writes if writes is not None else inputs-cached) for p in bands)
    unknown = [key for key,value in [('context_band',band),('cache_write_tokens',writes)] if value is None]
    return {'label':'proxy','api_model':mapping['api_model'],'source_url':table['source_url'],
            'observed_date':table['observed_date'],'mapping_version':table['version'],
            'method':'conservative additive cache-write envelope',
            'unknown_fields':unknown,'lower_nanodollars':lower,'upper_nanodollars':upper,
            'point_nanodollars':lower if not unknown else None}
