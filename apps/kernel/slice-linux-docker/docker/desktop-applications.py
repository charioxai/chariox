#!/usr/bin/env python3
"""MP-08/MP-11: installed desktop IDs only; never run a shell or activate D-Bus."""
import json, os, re, sys
from gi.repository import Gio, GLib

ID = re.compile(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,118}\.desktop\Z')

def installed(app):
    if not isinstance(app, Gio.DesktopAppInfo) or not ID.fullmatch(app.get_id() or ''):
        return False
    return (app.get_string('Type') == 'Application' and not app.get_is_hidden()
            and not app.get_nodisplay() and not app.get_boolean('Terminal')
            and bool(app.get_commandline()) and bool(app.get_name()))

def command(app):
    # GLib parses tokens, without expansion/execution. Field substitutions come
    # solely from the installed entry. The caller cannot supply files or URLs.
    ok, words = GLib.shell_parse_argv(app.get_commandline())
    if not ok or not words or len(words) > 64:
        raise ValueError('entry command')
    argv = []
    for word in words:
        if word in ('%f', '%F', '%u', '%U', '%d', '%D', '%n', '%N', '%v', '%m'):
            continue
        if word == '%i':
            icon = app.get_string('Icon')
            if icon: argv.extend(['--icon', icon])
            continue
        if word == '%c': word = app.get_name()
        elif word == '%k': word = app.get_filename()
        else:
            # Unsupported embedded field codes refuse, including %-encoded
            # application URLs. Literal %% is expanded once.
            if re.search(r'%(?!%)', word.replace('%%', '')): raise ValueError('field code')
            word = word.replace('%%', '%')
        if not isinstance(word, str) or '\x00' in word or len(word) > 4096:
            raise ValueError('entry argument')
        argv.append(word)
    if not argv or '=' in argv[0] or os.path.basename(argv[0]) in (
        'env', 'sh', 'bash', 'dash', 'zsh', 'fish', 'sudo', 'pkexec'):
        raise ValueError('entry executable')
    return argv

def request(value):
    if value == {'op': 'list'}:
        apps = sorted((a for a in Gio.AppInfo.get_all() if installed(a)), key=lambda a:a.get_id())
        result = []
        for app in apps:
            try: command(app)
            except (ValueError, GLib.Error): continue
            result.append({'application_id': app.get_id(), 'name': app.get_name()[:128]})
        return {'applications': result[:256], 'truncated': len(result) > 256}
    if not isinstance(value, dict) or set(value) != {'op', 'application_id'} or value['op'] != 'resolve':
        raise ValueError('request shape')
    app_id = value['application_id']
    if not isinstance(app_id, str) or not ID.fullmatch(app_id): raise ValueError('application ID')
    app = Gio.DesktopAppInfo.new(app_id)
    if app is None or not installed(app): raise ValueError('application unavailable')
    return {'application_id': app_id, 'name': app.get_name()[:128], 'argv': command(app)}

if __name__ == '__main__':
    try:
        data = sys.stdin.buffer.read(4097)
        if len(data) > 4096: raise ValueError('request limit')
        print(json.dumps(request(json.loads(data))))
    except Exception:
        # No descriptor, process environment or argument values reach diagnostics.
        print('MP-08: installed application refused', file=sys.stderr)
        sys.exit(1)
