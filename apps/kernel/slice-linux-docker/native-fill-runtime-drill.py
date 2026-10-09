#!/usr/bin/env python3
"""MP-08/MP-11: supplementary shipped-interpreter GTK fill/screenshot oracle.

Run inside the native runtime dependency image with an owned X11/AT-SPI desktop.
The value is a public canary; this does not establish provider/Vault acceptance.
"""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import time
from PIL import Image

ROOT = Path(__file__).parent / 'docker'
PYTHON = '/opt/chariox-selkies/bin/python'
VALUE = 'MP11-native-public-canary'


def command(args, **kwargs):
    return subprocess.run(args, check=True, capture_output=True, text=True, timeout=15, **kwargs)


def load(name):
    spec = importlib.util.spec_from_file_location(name, ROOT / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def main():
    evidence = Path(sys.argv[1])
    evidence.mkdir(parents=True, exist_ok=True)
    gtk = subprocess.Popen(['/usr/bin/python3', '-c', '''import gi
gi.require_version('Gtk','3.0')
from gi.repository import Gtk
w=Gtk.Window(title='MP11 native runtime fill');w.set_default_size(500,200)
b=Gtk.Box(orientation=Gtk.Orientation.VERTICAL);b.set_border_width(20)
b.pack_start(Gtk.Label(label='Ordinary desktop visible'),False,False,0)
entry=Gtk.Entry();b.pack_start(entry,False,False,0);w.add(b)
w.connect('destroy',Gtk.main_quit);w.show_all();entry.grab_focus();Gtk.main()
'''], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        window = None
        for _ in range(100):
            found = subprocess.run(['xdotool', 'search', '--name', '^MP11 native runtime fill$'], capture_output=True, text=True)
            if found.returncode == 0:
                window = found.stdout.splitlines()[0]
                break
            time.sleep(.05)
        assert window, 'MP-11 GTK window unavailable'
        command(['xdotool', 'windowactivate', '--sync', window])
        time.sleep(.5)
        target = json.loads(command([PYTHON, str(ROOT / 'slice-keyboard.py'), 'secret-target']).stdout)
        command([PYTHON, str(ROOT / 'slice-keyboard.py'), 'secret', json.dumps(target)], input=VALUE)
        native = load('native-fill-targets')
        boxes = native.regions()
        assert len(boxes) == 1, 'MP-11 actual helper interpreter failed to record the filled GTK entry'
        output = evidence / 'masked.png'
        command([PYTHON, str(ROOT / 'slice-observation-mask.py'), 'screenshot', str(output)], input=json.dumps({'unknown': False, 'targets': [], 'values': [VALUE]}))
        with Image.open(output) as image:
            x, y, width, height = boxes[0]
            assert image.crop((x, y, x+width, y+height)).getextrema() == ((0, 0), (0, 0), (0, 0)), 'MP-11 filled native field pixels escaped'
            assert image.getbbox(), 'MP-11 ordinary desktop must remain visible'
        print(json.dumps({'items': ['MP-08', 'MP-11'], 'interpreter': PYTHON, 'dpr': int(os.environ.get('GDK_SCALE', '1')), 'masked_fields': 1, 'result': 'PASS', 'limits': 'dependency-image physical helper regression, no provider/Vault/hosted acceptance'}))
    finally:
        if gtk.poll() is None and isinstance(gtk.pid, int) and gtk.pid > 1:
            gtk.terminate()
            try:
                gtk.wait(timeout=5)
            except subprocess.TimeoutExpired:
                if gtk.pid > 1:
                    gtk.kill()
                    gtk.wait(timeout=5)


if __name__ == '__main__':
    main()
