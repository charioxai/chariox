#!/usr/bin/env python3
"""Exact-receipt cleanup for one temporary Hetzner image builder.

Run on the existing OpenShip control host. Provider credentials stay in its
existing manager env file; never copy them to the disposable builder.
"""
import argparse
from datetime import datetime, timezone
from fractions import Fraction
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import stat
import time
import urllib.error
import urllib.request


AUTHORITY = Path('/etc/chariox-cloud/infrastructure-manager.env')
FORMAT = 'chariox-image-builder-cleanup/v1'
MAX_RUN_SECONDS = 180


def require(condition, message):
    if not condition:
        raise ValueError(message)


def timestamp(value):
    # Keep the Unix-seconds API used by the task creator's datetime.fromtimestamp.
    return float(exact_timestamp(value))


def exact_timestamp(value):
    match = re.fullmatch(r'(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2})(?:\.(\d{1,18}))?(?:Z|\+00:00)', value) \
        if isinstance(value, str) else None
    require(match is not None, 'UTC timestamp required')
    delta = datetime.fromisoformat(match[1] + '+00:00') - datetime(1970, 1, 1, tzinfo=timezone.utc)
    # Identity comparisons must not lose fractional precision through floats or
    # datetime's microsecond truncation. Equivalent trailing zeroes are harmless.
    fraction = match[2] or ''
    return Fraction(delta.days * 86400 + delta.seconds) + Fraction(int(fraction or '0'), 10 ** len(fraction))


def protected_bytes(path, maximum):
    path = Path(path)
    require(path.is_absolute() and str(path) == os.path.normpath(path), 'canonical absolute path required')
    for parent in path.parents:
        info = parent.lstat()
        require(stat.S_ISDIR(info.st_mode) and info.st_uid == 0 and info.st_mode & 0o022 == 0,
                'input parent must be a protected root-owned directory')
    fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    try:
        before = os.fstat(fd)
        require(stat.S_ISREG(before.st_mode) and before.st_uid == 0 and stat.S_IMODE(before.st_mode) == 0o600,
                'input must be a root-owned mode-0600 regular file')
        require(before.st_nlink == 1 and 0 < before.st_size <= maximum, 'input size or link count is unsafe')
        data = os.read(fd, maximum + 1)
        after = os.fstat(fd)
        named = path.lstat()
        identity = lambda item: (item.st_dev, item.st_ino, item.st_size, item.st_mtime_ns, item.st_ctime_ns)
        require(len(data) == before.st_size and identity(before) == identity(after) == identity(named),
                'input changed while reading')
        return data
    finally:
        os.close(fd)


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, 'duplicate JSON field')
        result[key] = value
    return result


def validate_receipt(value):
    require(set(value) == {'format', 'runId', 'sourceCommit', 'expiresAt', 'server', 'firewall', 'sshKey'},
            'unexpected cleanup receipt fields')
    require(value['format'] == FORMAT, 'unsupported cleanup receipt')
    require(re.fullmatch(r'[a-z][a-z0-9-]{7,47}', value['runId']) is not None, 'invalid run ID')
    require(re.fullmatch(r'[0-9a-f]{40}', value['sourceCommit']) is not None, 'invalid source commit')
    for kind, fields in [('server', {'id', 'name', 'created', 'ipv4', 'type', 'location', 'imageId'}),
                         ('firewall', {'id', 'name'}), ('sshKey', {'id', 'name', 'publicKeySha256'})]:
        item = value[kind]
        require(isinstance(item, dict) and set(item) == fields, 'invalid resource receipt fields')
        require(type(item['id']) is int and item['id'] > 0, 'invalid resource ID')
        suffix = {'server': '', 'firewall': '-ssh', 'sshKey': '-key'}[kind]
        require(item['name'] == value['runId'] + suffix, 'resource name does not match run')
    server = value['server']
    require(server['type'] == 'cpx22' and server['location'] == 'fsn1', 'unexpected preparation host class')
    require(type(server['imageId']) is int and server['imageId'] > 0, 'invalid base image ID')
    import ipaddress
    require(ipaddress.ip_address(server['ipv4']).version == 4, 'invalid builder IPv4')
    require(0 < exact_timestamp(value['expiresAt']) - exact_timestamp(server['created']) <= 7200,
            'cleanup deadline must be within two hours of server creation')
    require(re.fullmatch(r'[0-9a-f]{64}', value['sshKey']['publicKeySha256']) is not None,
            'invalid public key fingerprint')
    return value


def labels(receipt):
    return {'chariox.dev/managed-image-builder': 'true',
            'chariox.dev/task': receipt['runId'],
            'chariox.dev/source-revision': receipt['sourceCommit']}


def validate_resource(resource, expected, receipt):
    require(resource.get('id') == expected['id'] and resource.get('name') == expected['name']
            and resource.get('labels') == labels(receipt), 'resource identity or labels changed')


def validate_server(server, receipt):
    expected = receipt['server']
    validate_resource(server, expected, receipt)
    require(exact_timestamp(server.get('created')) == exact_timestamp(expected['created']), 'server creation identity changed')
    locations = [item.get('name') for item in [server.get('location'),
                 (server.get('datacenter') or {}).get('location')] if item is not None]
    require(server.get('server_type', {}).get('name') == expected['type']
            and locations and all(item == expected['location'] for item in locations)
            and (server.get('image') or {}).get('id') == expected['imageId'], 'server placement or base image changed')
    require(not server.get('volumes') and not server.get('rescue_enabled') and not server.get('backup_window')
            and server.get('protection', {}).get('delete') is False, 'server has unexpected protected state')
    network = server.get('public_net', {})
    require((network.get('ipv4') or {}).get('ip') == expected['ipv4'], 'server IPv4 changed')
    require([item.get('id') for item in network.get('firewalls', [])] == [receipt['firewall']['id']],
            'server firewall binding changed')


def validate_firewall(firewall, receipt, server_absent=False):
    validate_resource(firewall, receipt['firewall'], receipt)
    rules = firewall.get('rules')
    require(isinstance(rules, list) and len(rules) == 1, 'temporary firewall rule count changed')
    rule = rules[0]
    require(rule.get('direction') == 'in' and rule.get('protocol') == 'tcp' and rule.get('port') == '22'
            and rule.get('source_ips') == ['51.38.82.47/32']
            and not rule.get('destination_ips'), 'temporary firewall scope changed')
    attached = firewall.get('applied_to')
    require(isinstance(attached, list), 'firewall attachment inventory missing')
    if server_absent:
        require(not attached, 'temporary firewall is still attached')
    else:
        require(all(item.get('type') == 'server' and item.get('server', {}).get('id') == receipt['server']['id']
                    for item in attached), 'temporary firewall is shared with another resource')


def validate_key(key, receipt):
    validate_resource(key, receipt['sshKey'], receipt)
    parts = key.get('public_key', '').split()
    require(len(parts) in (2, 3) and parts[0] == 'ssh-ed25519', 'temporary public key type changed')
    digest = hashlib.sha256(' '.join(parts[:2]).encode()).hexdigest()
    require(digest == receipt['sshKey']['publicKeySha256'], 'temporary public key changed')


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise ValueError('provider redirect refused')


class Provider:
    def __init__(self, token, deadline):
        self.token, self.deadline = token, deadline
        self.opener = urllib.request.build_opener(NoRedirect)

    def request(self, route, method='GET', allow_absent=False):
        require(re.fullmatch(r'(servers|firewalls|ssh_keys|actions)/[1-9][0-9]*', route) is not None,
                'unsupported cleanup API path')
        require(method in ('GET', 'DELETE') and not (method == 'DELETE' and route.startswith('actions/')),
                'unsupported cleanup API operation')
        remaining = self.deadline - time.monotonic()
        require(remaining > 0, 'cleanup attempt deadline expired')
        req = urllib.request.Request('https://api.hetzner.cloud/v1/' + route,
                                     headers={'Authorization': 'Bearer ' + self.token}, method=method)
        try:
            with self.opener.open(req, timeout=min(20, remaining)) as response:
                body = response.read(1024 * 1024 + 1)
                require(len(body) <= 1024 * 1024, 'provider response exceeds limit')
                return json.loads(body) if body else {}
        except urllib.error.HTTPError as error:
            if method == 'GET' and allow_absent and error.code == 404:
                return None
            raise ValueError(f'provider {method} {route} failed HTTP {error.code}') from None
        except urllib.error.URLError:
            raise ValueError('provider connection failed; inspect exact receipts before retry') from None


def get_resource(api, collection, key, expected):
    result = api.request(f'{collection}/{expected["id"]}', allow_absent=True)
    if result is None:
        return None
    require(isinstance(result, dict) and isinstance(result.get(key), dict), 'provider resource response is malformed')
    return result[key]


def cleanup(api, receipt, apply=False, sleep=time.sleep):
    """Idempotent only after fresh exact-ID readback, never blind DELETE retries."""
    server = get_resource(api, 'servers', 'server', receipt['server'])
    firewall = get_resource(api, 'firewalls', 'firewall', receipt['firewall'])
    key = get_resource(api, 'ssh_keys', 'ssh_key', receipt['sshKey'])
    if server:
        validate_server(server, receipt)
        require(firewall is not None and key is not None, 'live builder is missing owned access assets')
    if firewall:
        validate_firewall(firewall, receipt, server_absent=server is None)
    if key:
        validate_key(key, receipt)
    if not apply:
        return {'checked': True, 'serverPresent': server is not None, 'firewallPresent': firewall is not None,
                'sshKeyPresent': key is not None}
    if server:
        result = api.request(f'servers/{receipt["server"]["id"]}', 'DELETE')
        action = result.get('action', {})
        require(type(action.get('id')) is int and action['id'] > 0, 'delete response lacks action receipt')
        action_id = action['id']
        for attempt in range(60):
            action = api.request(f'actions/{action_id}').get('action', {})
            require(action.get('id') == action_id, 'provider delete action identity changed')
            require(action.get('command') == 'delete_server', 'unexpected provider delete action')
            require(action.get('resources') == [{'id': receipt['server']['id'], 'type': 'server'}],
                    'delete action targets unexpected resources')
            require(action.get('status') in ('running', 'success') and not action.get('error'), 'server deletion failed')
            if action['status'] == 'success':
                break
            sleep(1)
        else:
            raise ValueError('server delete action did not settle')
    require(get_resource(api, 'servers', 'server', receipt['server']) is None,
            'server absence is unproved; preserving access assets')
    # Re-read every remaining resource after server deletion, including shared use.
    for collection, key_name, expected, verify in [
        ('firewalls', 'firewall', receipt['firewall'], lambda value: validate_firewall(value, receipt, True)),
        ('ssh_keys', 'ssh_key', receipt['sshKey'], lambda value: validate_key(value, receipt)),
    ]:
        resource = get_resource(api, collection, key_name, expected)
        if resource:
            verify(resource)
            api.request(f'{collection}/{expected["id"]}', 'DELETE')
        require(get_resource(api, collection, key_name, expected) is None, 'resource deletion is not confirmed')
    return {'cleaned': True, 'serverId': receipt['server']['id'], 'firewallId': receipt['firewall']['id'],
            'sshKeyId': receipt['sshKey']['id']}


def authority_token():
    values = []
    for line in protected_bytes(AUTHORITY, 1024 * 1024).decode().splitlines():
        if line.startswith('CHARIOX_HETZNER_TOKEN='):
            parsed = shlex.split(line.split('=', 1)[1], comments=False, posix=True)
            require(len(parsed) == 1, 'provider authority is malformed')
            values.append(parsed[0])
    require(len(values) == 1 and re.fullmatch(r'[A-Za-z0-9_-]{20,256}', values[0]) is not None,
            'existing provider authority unavailable')
    return values[0]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=['check', 'expire', 'cleanup'])
    parser.add_argument('--receipt', required=True)
    args = parser.parse_args()
    require(os.geteuid() == 0, 'run on the OpenShip authority host as root')
    receipt = validate_receipt(json.loads(protected_bytes(args.receipt, 16384), object_pairs_hook=unique_object))
    if args.mode == 'expire':
        require(time.time() >= timestamp(receipt['expiresAt']), 'cleanup deadline has not arrived')
    api = Provider(authority_token(), time.monotonic() + MAX_RUN_SECONDS)
    print(json.dumps(cleanup(api, receipt, apply=args.mode != 'check'), sort_keys=True))


if __name__ == '__main__':
    try:
        main()
    except Exception as error:
        # Never print HTTP bodies, request headers, credentials or receipt data.
        message = str(error) if isinstance(error, ValueError) else type(error).__name__
        print('Image builder cleanup failed: ' + message[:240], file=__import__('sys').stderr)
        raise SystemExit(1)
