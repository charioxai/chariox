#!/usr/bin/env python3
"""MP-08 / MP-11: public GTK target and synthetic password protection fixture."""
import gi
import sys
from pathlib import Path
gi.require_version('Gtk','3.0')
from gi.repository import Gtk, GLib
root=Path(sys.argv[1])
window=Gtk.Window(title='Chariox public AT-SPI fixture')
window.set_default_size(500,250)
box=Gtk.Box(orientation=Gtk.Orientation.VERTICAL,spacing=10)
button=Gtk.Button(label='Public action')
button.connect('clicked',lambda _: (root/'clicked').write_text('public effect'))
box.pack_start(button,True,True,0)
password=Gtk.Entry();password.set_visibility(False);password.set_text('synthetic-password-canary')
box.pack_start(password,True,True,0)
if len(sys.argv)>2 and sys.argv[2]=='plain-entry':
    # A visible entry for the Vault-fill masking check; the magenta text makes its glyph pixels countable.
    plain=Gtk.Entry();plain.get_accessible().set_name('Plain text');plain.set_name('native-plain')
    css=Gtk.CssProvider();css.load_from_data(b'#native-plain { color: #ff00ff; caret-color: transparent; }')
    plain.get_style_context().add_provider(css,Gtk.STYLE_PROVIDER_PRIORITY_APPLICATION)
    box.pack_start(plain,True,True,0)
window.add(box);window.connect('destroy',Gtk.main_quit);window.show_all()
def tick():
    if (root/'change').exists():button.set_label('Changed public action')
    return True
GLib.timeout_add(50,tick)
Gtk.main()
