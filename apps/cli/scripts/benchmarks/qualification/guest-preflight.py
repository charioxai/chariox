#!/usr/bin/env python3
"""MP-08 / MP-10 / MP-11 read-only canonical OSWorld preparation; never boots guests."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket

PINS = {
 'OSWorld-evaluation_examples-test_all.json': '9ebc5187cbd727ef26c24626820076b102fff812863c640a67c467fea9542ab5',
 'OSWorld-evaluation_examples-test_nogdrive.json': 'fcb9497e93a8986407345d3012c872c9c2fed253420730fbea64ccfcace67dbb',
 'OSWorld-V2-benchmark_releases-osworld-v2.1.json': '978686683bdf9f946703c1d316cbbd29c0e3ea97396e48d940ee92c597806ecf',
}


def prepare(root):
    data = {}
    for name, expected in PINS.items():
        source = (root/name).read_bytes()
        if hashlib.sha256(source).hexdigest() != expected:
            raise ValueError('MP-10 canonical manifest identity drift')
        data[name] = json.loads(source)
    full = data['OSWorld-evaluation_examples-test_all.json']
    nogdrive = data['OSWorld-evaluation_examples-test_nogdrive.json']
    flat = lambda d: {(app, task) for app, tasks in d.items() for task in tasks}
    if len(flat(full)) != 369 or len(flat(nogdrive)) != 361 or not flat(nogdrive) < flat(full):
        raise ValueError('MP-10 official denominator drift')
    v2 = data['OSWorld-V2-benchmark_releases-osworld-v2.1.json']
    return {'v1': {'sourceCommit': 'b138d348256078fa634fc3b73567a7337c793e6b', 'full': 369,
                   'withoutDrive': 361, 'excludedDriveCount': len(flat(full)-flat(nogdrive))},
            'v2': {'sourceCommit': 'acdd3493808e716825975b0f0208194bb2faf3c3',
                   'release': v2['release'], 'denominator': v2['task_hash_manifest']['task_count'],
                   'tasks': v2['tasks'], 'assets': v2['assets'], 'website': v2['website_code'],
                   'taskHashManifest': v2['task_hash_manifest'],
                   'guest': v2['provider_images']['docker']['ubuntu'],
                   'awsGuest': v2['provider_images']['aws']['ubuntu'],
                   'knownLimitations': v2['verification']['limitations']}}


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--upstream', required=True, type=Path)
    ap.add_argument('--output', required=True, type=Path)
    args = ap.parse_args()
    from qualification_paths import require_external
    require_external(args.output)
    pins = prepare(args.upstream)
    mem = {}
    for line in Path('/proc/meminfo').read_text().splitlines():
        key, value = line.split(':', 1)
        if key in {'MemTotal', 'MemAvailable', 'SwapTotal', 'SwapFree'}:
            parts = value.split()
            if len(parts) != 2 or parts[1] != 'kB' or not parts[0].isdigit():
                raise ValueError('MP-10 invalid resource sample')
            mem[key] = int(parts[0])*1024
    disk = shutil.disk_usage('/')
    kvm = Path('/dev/kvm').exists() and os.access('/dev/kvm', os.R_OK | os.W_OK)
    qemu = shutil.which('qemu-system-x86_64') is not None
    reasons = ['owner_approved_host_guest_track_missing', 'kernel_room_canonical_guest_binding_unqualified',
               'guest_assets_not_materialized_or_verified', 'human_judge_access_unconfirmed']
    if not kvm: reasons.append('KVM_unavailable_TCG_RED')
    if not qemu: reasons.append('QEMU_unavailable')
    if mem['MemAvailable'] < 16*2**30+8*2**30: reasons.append('forecast_guest_peak_memory_floor')
    if disk.free < 10*2**30+64*2**30: reasons.append('forecast_guest_disk_floor')
    report = {'mpItems': ['MP-08','MP-10','MP-11'], 'status': 'RED', 'scoredCampaign': False,
              'machine': socket.gethostname(), 'pins': pins, 'manifestHashes': PINS,
              'resources': {**mem, 'diskFreeBytes': disk.free},
              'forecastOnly': {'guestMemoryBytes': 8*2**30, 'guestTemporaryDiskBytes': 64*2**30,
                               'memoryFloorBytes': 16*2**30, 'diskFloorBytes': 10*2**30},
              'qualification': {'kvmAccessible': kvm, 'qemuInstalled': qemu, 'vmBooted': False,
                                'canonicalComputerBindingAccepted': False}, 'firstFailingSeam': reasons[0], 'reasons': reasons}
    with args.output.open('x') as f: json.dump(report, f, indent=2); f.write('\n')
    print('MP-08 / MP-10 / MP-11 guest readiness RED: owner-approved host/guest/track and product binding required; no VM boot')
    return 1

if __name__ == '__main__': raise SystemExit(main())
