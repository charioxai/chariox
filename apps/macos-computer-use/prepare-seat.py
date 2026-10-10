#!/usr/bin/env python3
"""Launch only the retained public fixture and print kernel bootstrap scope."""
import argparse
import importlib.util
import json
from pathlib import Path
import re
import subprocess
import time

spec = importlib.util.spec_from_file_location('d', Path(__file__).with_name('text-click-drill.py'))
d = importlib.util.module_from_spec(spec)
spec.loader.exec_module(d)

def make_scope(output, state):
    identity = re.search(r'fixturePID=(\d+) windowID=(\d+)', output)
    scale = re.search(r'fixture geometry[^\n]*scale=([12](?:\.0)?)\b', output)
    if not identity or not scale or state['frontmostPID'] != int(identity[1]):
        raise RuntimeError('fixture-binding-unavailable')
    pid, window = map(int, identity.groups())
    matches = [w for w in state['windows'] if w['pid'] == pid and w['windowID'] == window]
    if len(matches) != 1: raise RuntimeError('fixture-geometry-unavailable')
    probes = {row[0]: row for row in d.fixture_probes(output)}
    point, revoke = probes['line5-after-r'], probes['line5-before-r']
    bounds = matches[0]['bounds']
    for row in [point, revoke]:
        if not (bounds['X'] <= float(row[1]) < bounds['X'] + bounds['Width'] and
                bounds['Y'] <= float(row[2]) < bounds['Y'] + bounds['Height']):
            raise RuntimeError('fixture-point-outside-window')
    return {'pid': pid, 'window': window, 'owner_window': False, 'target': 'text',
        'bounds': bounds, 'scale': float(scale[1]), 'point': {'x': float(point[1]), 'y': float(point[2])},
        'expected_location': int(point[3]), 'revoke_point': {'x': float(revoke[1]), 'y': float(revoke[2])},
        'revoke_location': int(revoke[3])}

def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--evidence', type=Path, required=True)
    args = parser.parse_args()
    evidence = args.evidence.resolve()
    if not evidence.is_relative_to(Path.home() / '.codex/evidence'):
        raise RuntimeError('evidence-must-be-under-owner-evidence-root')
    evidence.mkdir(parents=True, exist_ok=True)
    d.wait_for_gui(); d.require_gui()
    subprocess.run(['open', '-n', '--stdout', str(evidence / 'fixture.out'), '--stderr', str(evidence / 'fixture.err'),
                    str(d.BUILD / 'Chariox Computer Fixture.app'), '--args', '--unattended', '--text-drill'], check=True)
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        output = d.read(evidence, 'fixture.out')
        if 'fixture geometry' in output:
            scope = make_scope(output, d.desktop())
            (evidence / 'scope.json').write_text(json.dumps(scope, indent=2) + '\n')
            print(json.dumps({key: scope[key] for key in ['pid', 'window', 'owner_window', 'target']}))
            return
        time.sleep(.05)
    raise RuntimeError('fixture-start-timeout')
if __name__ == '__main__': main()
