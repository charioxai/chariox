"""MP-08 / MP-10: settle only an image/label-verified owned Docker container."""
import subprocess
import time


def stop_owned_container(name, image, owner, inspect, docker, timeout=20):
    code, actual = inspect(name, '{{.Image}}')
    if code:
        return {'name': name, 'alreadyAbsent': True, 'absent': True}
    if actual != image:
        raise ValueError('owned container image mismatch')
    code, actual_owner = inspect(name, '{{index .Config.Labels "dev.chariox.local-web-owner"}}')
    if code or actual_owner != owner:
        raise ValueError('owned container label mismatch')
    subprocess.run([*docker, 'stop', '--time', '15', name], stdout=subprocess.PIPE,
                   stderr=subprocess.PIPE, timeout=25, check=True)
    deadline = time.monotonic() + timeout
    while True:
        code, identity = inspect(name, '{{.Id}}')
        if code:
            return {'name': name, 'absent': True}
        if time.monotonic() >= deadline:
            return {'name': name, 'absent': False, 'error': 'auto_removal_timeout'}
        time.sleep(.1)
