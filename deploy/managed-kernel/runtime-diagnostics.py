#!/usr/bin/env python3
"""MP-07/MP-08/MP-10/MP-11: campaign-only public diagnostic shipping.

The guest fsyncs records before POST; the observer fsyncs before acknowledging
their SHA256. This is observation, never an update/liveness authority. No tokens,
credentials, prompts, provider output, general log files or config are read.
"""
import argparse
import fcntl
import hashlib
import http.server
import json
import os
import pathlib
import re
import stat
import tempfile
import time
import urllib.parse
import urllib.request

EVENTS = frozenset(('prompt_dispatch provider_dispatch_start provider_dispatch_returned provider_dispatch_failed heartbeat_sent heartbeat_failed cloud_presence_start cloud_presence_acknowledged cloud_presence_failed update_poll '
    'update_poll_failed update_unit_running update_recovery update_pending update_applied '
    'update_failed update_cloud_acknowledged update_download update_downloaded '
    'update_unit_starting update_unit_started prepared stopped activated committed '
    'rolled_back observer_flush update_unexpected_exit '
    'activation_builder_pin_start activation_home_migration_start activation_receipt_start '
    'builder_pin_journal_start builder_pin_journal_returned builder_pin_runtime_start '
    'builder_pin_runtime_returned builder_pin_compare_start builder_pin_compare_returned '
    'builder_pin_atomic_start builder_pin_atomic_returned builder_pin_failed '
    'builder_pin_journal_directory_failed builder_pin_journal_ancestor_failed builder_pin_journal_previous_missing builder_pin_journal_previous_file_failed builder_pin_journal_previous_digest_failed builder_pin_journal_previous_release_ancestor_failed builder_pin_journal_previous_release_file_failed builder_pin_journal_previous_compare_failed builder_pin_journal_target_missing builder_pin_journal_target_file_failed builder_pin_journal_target_digest_failed builder_pin_journal_target_release_ancestor_failed builder_pin_journal_target_release_file_failed builder_pin_journal_target_compare_failed builder_pin_runtime_ancestor_failed builder_pin_runtime_file_failed builder_pin_role_failed builder_pin_compare_failed builder_pin_atomic_failed '
    'activation_release_override_start activation_app_prepare_start activation_current_link_start '
    'activation_data_volume_links_start activation_app_storage_start activation_slice_facade_start '
    'activation_slice_facade_check_start '
    'image_prepare_start image_prepare_packages image_prepare_pin image_prepare_providers '
    'image_prepare_provider_probe image_prepare_rootless image_prepare_pull image_prepare_build '
    'image_prepare_freeze image_prepare_complete image_prepare_failed '
    'image_install_start image_install_verify image_install_pin image_install_publish '
    'image_install_activate image_install_complete').split())
MAX_RECORD = 1024
MAX_JOURNAL = 8 * 1024 * 1024


def private_directory(path):
    info = path.lstat()
    if not path.is_absolute() or not stat.S_ISDIR(info.st_mode) or stat.S_IMODE(info.st_mode) != 0o700:
        raise ValueError('MP-11 private diagnostic directory required')
    return info.st_uid


def canonical(value):
    if (type(value) is not dict or set(value) != {'schema', 'atMs', 'pid', 'event'}
            or type(value['schema']) is not int or value['schema'] != 1
            or type(value['pid']) is not int or value['pid'] <= 1
            or type(value['atMs']) is not int or not 0 < value['atMs'] < 2**63
            or type(value['event']) is not str or value['event'] not in EVENTS):
        raise ValueError('MP-11 non-public diagnostic record rejected')
    return (json.dumps(value, sort_keys=True, separators=(',', ':')) + '\n').encode()


def sync_directory(directory):
    descriptor = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def accept(directory, raw):
    owner = private_directory(directory)
    if len(raw) > MAX_RECORD:
        raise ValueError('MP-11 record exceeds bound')
    data = canonical(json.loads(raw))
    digest = hashlib.sha256(data).hexdigest()
    path = directory / (digest + '.json')
    lock = os.open(directory, os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        # Serialize quota/validation/publication across cooperating receivers.
        # A process crash releases the directory lock without a stale lock file.
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        except FileNotFoundError:
            if sum(1 for _ in directory.glob('*.json')) >= 10000:
                raise ValueError('MP-09 observer record quota exceeded')
            descriptor, temporary = tempfile.mkstemp(prefix='.' + digest + '.', suffix='.tmp', dir=directory)
            try:
                with os.fdopen(descriptor, 'wb') as stream:
                    stream.write(data)
                    stream.flush()
                    os.fsync(stream.fileno())
                # Incomplete writes and abandoned staging files never become receipts.
                os.replace(temporary, path)
            finally:
                pathlib.Path(temporary).unlink(missing_ok=True)
        else:
            with os.fdopen(descriptor, 'rb') as stream:
                info = os.fstat(stream.fileno())
                if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_uid != owner or stream.read(MAX_RECORD + 1) != data:
                    raise ValueError('MP-11 observer collision')
                os.fsync(stream.fileno())
        os.fsync(lock)
    finally:
        os.close(lock)
    return digest


def event(directory, name):
    owner = private_directory(directory)
    data = canonical({'schema': 1, 'atMs': time.time_ns() // 1_000_000, 'pid': os.getpid(), 'event': name})
    path = directory / ('runtime-' + str(os.getpid()) + '-upgrade.jsonl')
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'ab') as stream:
        info = os.fstat(stream.fileno())
        if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1
                or stat.S_IMODE(info.st_mode) != 0o600 or info.st_size + len(data) > MAX_JOURNAL):
            raise ValueError('MP-11 unsafe or full diagnostic journal')
        if os.geteuid() == 0 and info.st_uid != owner:
            os.fchown(stream.fileno(), owner, -1)
        elif info.st_uid != owner:
            raise ValueError('MP-11 diagnostic journal owner mismatch')
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    sync_directory(directory)


def records(directory):
    owner = private_directory(directory)
    for path in sorted(directory.glob('runtime-*.jsonl')):
        if not re.fullmatch(r'runtime-[0-9]+-(?:[0-9]+|upgrade)\.jsonl', path.name):
            raise ValueError('MP-11 unexpected diagnostic journal name')
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(descriptor, 'rb') as stream:
            info = os.fstat(stream.fileno())
            if (not stat.S_ISREG(info.st_mode) or info.st_nlink != 1 or info.st_uid != owner
                    or stat.S_IMODE(info.st_mode) != 0o600 or info.st_size > MAX_JOURNAL):
                raise ValueError('MP-11 unsafe diagnostic journal')
            # Read a bounded snapshot; a crash-truncated record is a collection failure.
            for line in stream.read(MAX_JOURNAL + 1).splitlines(keepends=True):
                if not line.endswith(b'\n') or len(line) > MAX_RECORD:
                    raise ValueError('MP-10 incomplete diagnostic record')
                yield canonical(json.loads(line))


def ship(directory, url, acknowledged, post=None):
    parsed = urllib.parse.urlsplit(url)
    if parsed.scheme != 'https' or not parsed.hostname or parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError('MP-11 HTTPS observer URL without credentials required')
    def send(raw):
        request = urllib.request.Request(url, raw, {'Content-Type': 'application/json'}, method='POST')
        # No redirects: never forward evidence to an unbound endpoint.
        class NoRedirect(urllib.request.HTTPRedirectHandler):
            def redirect_request(self, *args, **kwargs):
                return None
        with urllib.request.build_opener(NoRedirect).open(request, timeout=5) as response:
            return json.loads(response.read(MAX_RECORD + 1))
    post = post or send
    observed = []
    for raw in records(directory):
        digest = hashlib.sha256(raw).hexdigest()
        observed.append(digest)
        if digest not in acknowledged:
            result = post(raw)
            if result != {'sha256': digest}:
                raise ValueError('MP-10 observer did not acknowledge exact record')
            acknowledged.add(digest)
    return {'MP': ['MP-07', 'MP-08', 'MP-10', 'MP-11'], 'status': 'ACKNOWLEDGED_SNAPSHOT',
            'records': len(set(observed)), 'snapshotSha256': hashlib.sha256('\n'.join(sorted(set(observed))).encode()).hexdigest(),
            'atMs': time.time_ns() // 1_000_000, 'acceptance': False}


def receive(directory, port, path):
    private_directory(directory)
    if not re.fullmatch(r'/path1-diagnostics/[a-z0-9-]{8,80}', path):
        raise ValueError('MP-11 exact campaign observer path required')
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args):
            pass
        def do_POST(self):
            try:
                length = int(self.headers.get('Content-Length', '0'))
                if self.path != path or not 0 < length <= MAX_RECORD or self.headers.get('Transfer-Encoding'):
                    raise ValueError('MP-11 bounded exact route required')
                data = self.rfile.read(length)
                if len(data) != length:
                    raise ValueError('MP-11 truncated HTTP record')
                result = json.dumps({'sha256': accept(directory, data)}).encode()
            except (ValueError, OSError):
                self.send_error(400)
                return
            self.send_response(200)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(result)))
            self.end_headers()
            self.wfile.write(result)
    # Only the own-stack TLS proxy can reach this listener. Public event receipts
    # are supplementary evidence, never authenticated guest/Cloud authority.
    class Server(http.server.HTTPServer):
        def get_request(self):
            connection, address = super().get_request()
            connection.settimeout(5)
            return connection, address
    with Server(('127.0.0.1', port), Handler) as server:
        server.serve_forever()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['event', 'ship', 'receive'])
    parser.add_argument('--directory', type=pathlib.Path, required=True)
    parser.add_argument('--event', choices=sorted(EVENTS))
    parser.add_argument('--observer-url')
    parser.add_argument('--once', action='store_true')
    parser.add_argument('--port', type=int)
    parser.add_argument('--path')
    args = parser.parse_args()
    if args.mode == 'event':
        event(args.directory, args.event)
    elif args.mode == 'receive':
        receive(args.directory, args.port, args.path)
    else:
        acknowledged = set()
        while True:
            try:
                result = ship(args.directory, args.observer_url, acknowledged)
                if args.once:
                    print(json.dumps(result))
                    return
            except (ValueError, OSError):
                if args.once:
                    raise
            time.sleep(5)


if __name__ == '__main__':
    try:
        main()
    except (ValueError, OSError):
        raise SystemExit('MP-07/MP-10/MP-11 diagnostic collection failed')
