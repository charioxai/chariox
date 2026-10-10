"""Bounded WindowServer cleanup is part of the unattended drill verdict."""
import time

def finish_cleanup(report, closed, now=time.monotonic, wait=time.sleep):
    deadline = now() + 2
    while not closed() and now() < deadline:
        wait(.05)
    report['owned_window_closed'] = closed()
    if not report['owned_window_closed']:
        report['status'] = 'FAIL'
