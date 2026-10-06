"""MP-11: guard real harness process objects before any signal is sent."""
from contract import safe_pid


def guarded_signal(original):
    def send(process, *args, **kwargs):
        safe_pid(process.pid)
        return original(process, *args, **kwargs)
    return send


def install():
    import asyncio
    import subprocess
    # These handles belong to subprocesses created by this Harbor process.
    # Preserve the official harness behavior for valid child PIDs.
    for kind, names in [(asyncio.subprocess.Process, ['terminate','kill']),
                        (subprocess.Popen, ['send_signal'])]:
        for name in names:
            original = getattr(kind, name)
            if not getattr(original, '_chariox_pid_guard', False):
                wrapper = guarded_signal(original)
                wrapper._chariox_pid_guard = True
                setattr(kind, name, wrapper)
