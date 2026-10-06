#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11 H1: unchanged pinned NetworkEvent navigation classifier."""
import argparse
import ast
import hashlib
import json
from pathlib import Path
from types import MappingProxyType
from typing import Any, Self
from urllib.parse import unquote, urlparse
from pydantic import BaseModel, ConfigDict, field_serializer

ap = argparse.ArgumentParser(description=__doc__)
ap.add_argument('--source', required=True, type=Path)
ap.add_argument('--har', required=True, type=Path)
ap.add_argument('--output', required=True, type=Path)
args = ap.parse_args()
from qualification_paths import require_external
require_external(args.output)
source = args.source.read_bytes()
expected = '5b31038716a9701f0fcc03e7b4a48acb79a54a59f5c84dae370fd9f8358d631d'
assert hashlib.sha256(source).hexdigest() == expected, 'official classifier source drift'
# Execute the exact NetworkEvent class AST. Trace loader utilities are unrelated
# to classification and are not replaced or used; no official method is edited.
node = next(n for n in ast.parse(source).body if isinstance(n, ast.ClassDef) and n.name == 'NetworkEvent')
exec(compile(ast.Module(body=[node], type_ignores=[]), str(args.source), 'exec'), globals())
har = json.loads(args.har.read_bytes())
assert har['log']['_capture']['flushAcknowledged'] is True
rows = []
for entry in har['log']['entries']:
    observed = NetworkEvent(data=MappingProxyType(entry)).is_navigation_event
    removed = json.loads(json.dumps(entry))
    removed['request']['headers'] = [h for h in removed['request']['headers'] if h['name'].lower() != 'accept']
    without_accept = NetworkEvent(data=MappingProxyType(removed)).is_navigation_event
    rows.append({'navigation': observed, 'withoutObservedAccept': without_accept,
                 'headersSource': entry['_requestMetadata']['headersSource']})
assert any(row['navigation'] and not row['withoutObservedAccept'] for row in rows)
report = {'mpItems': ['MP-08','MP-10','MP-11'], 'status': 'PASS', 'scoredCampaign': False,
          'sourceCommit': '6473f72db5dcefc97b5725b59e734504edc28a21', 'sourceSha256': expected,
          'harSha256': hashlib.sha256(args.har.read_bytes()).hexdigest(), 'events': rows,
          'limits': 'Unchanged official class, retained first-party Chrome HAR; not full WebArena evaluator acceptance.'}
with args.output.open('x') as f: json.dump(report, f, indent=2); f.write('\n')
print('MP-08 / MP-10 / MP-11 H1 PASS: actual ExtraInfo Accept enables unchanged official navigation classifier')
