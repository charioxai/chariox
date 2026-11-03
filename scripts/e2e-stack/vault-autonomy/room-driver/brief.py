"""Compact view of the agent's recent provider output and tool calls from events.jsonl: brief.py [count] [iso-prefix]."""
import json
import os
import re
import sys

count = int(sys.argv[1]) if len(sys.argv) > 1 else 25
since = sys.argv[2] if len(sys.argv) > 2 else ''
out = []
for raw in open(os.path.join(os.environ['ROOM_DIR'], 'events.jsonl')):
    row = json.loads(raw)
    record, kind, stamp = row['rec'], row['rec'].get('kind'), row['ts'][11:19]
    if row['ts'] < since or 'bytes' not in record:
        continue
    text = bytes(record['bytes']).decode('utf8', 'replace')
    if kind in ('provider_output', 'prompt_echo'):
        key = (kind, record.get('merge_key'))
        if out and out[-1][0] == key:
            out[-1][2] += text
        else:
            out.append([key, stamp, text])
    elif kind == 'provider_tool':
        tool = re.search(r'"tool":"([^"]+)"', text)
        failed = '"status":"error"' in text or '"error":' in text[:400]
        arguments = re.search(r'"input":(\{.*?\}),"(?:output|status)"', text)
        out.append([(kind, None), stamp, f"{'ERR ' if failed else ''}{tool.group(1) if tool else '?'} {(arguments.group(1) if arguments else '')[:150]}"])
for key, stamp, text in out[-count:]:
    print(stamp, key[0][:8], '|', text.replace('\n', ' / ')[:260])
