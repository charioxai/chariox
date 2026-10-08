"""MP-08 / MP-11: owned virtual displays use their shared abstract X11 socket.

Xvfb has a private filesystem /tmp. Python Xlib otherwise prefers a host
filesystem socket with the same display number, which can belong to another
kernel. Keep normal Xauthority authentication; never fall back to that socket.
"""
import importlib
import os
import socket
import sys
import threading


def open_display(display_module):
    if sys.platform != 'linux' or os.environ.get('CHARIOX_OWNED_VIRTUAL_DISPLAY') != '1':
        return display_module.Display()
    transport=importlib.import_module(display_module.__package__+'.support.unix_connect')
    # Helpers can load this file by path independently. Share the lock on the
    # actual library transport so nested native/clipboard reads serialize.
    lock=transport.__dict__.setdefault('_chariox_owned_socket_lock',threading.RLock())
    with lock:
        original=transport.get_socket
        def owned_socket(name,protocol,host,number):
            if protocol not in (None,'unix') or host not in ('','unix',None):
                return original(name,protocol,host,number)
            if type(number) is not int or number<0 or number>65535:
                raise ValueError('MP-11: invalid owned display number')
            connection=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM)
            try:
                connection.settimeout(2)
                connection.connect('\0/tmp/.X11-unix/X'+str(number))
                connection.settimeout(None)
                connection.set_inheritable(False)
                return connection
            except BaseException:
                connection.close()
                raise
        transport.get_socket=owned_socket
        try:return display_module.Display()
        finally:transport.get_socket=original
