"""Private Selkies token table. This does not authorize Chariox Room viewers."""

from contextlib import contextmanager
import fcntl
import importlib.util
import json
import os
from pathlib import Path
import secrets
import urllib.request

import psutil


spec = importlib.util.spec_from_file_location("selkies_lifecycle", Path(__file__).with_name("slice-selkies.py"))
lifecycle = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lifecycle)


class PrivateStreamError(RuntimeError):
    """Only fixed, non-payload diagnostics may use this exception."""


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, message, headers, new_url):
        return None


@contextmanager
def locked_state():
    directory = lifecycle.state_directory()
    descriptor = os.open(directory / "lifecycle.lock", os.O_RDWR | os.O_CREAT, 0o600)
    with os.fdopen(descriptor, "w") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        yield directory, lifecycle.read_state(directory)


def publish(directory, record):
    viewers = {token: owner for token, owner in record.get("viewers", {}).items()
               if lifecycle.owned_process(owner) is not None}
    record["viewers"] = viewers
    # Persist first so an uncertain HTTP outcome can be reconciled by close or
    # the next stream. Only the exact live streamer generation is mutated.
    lifecycle.write_state(directory, record)
    request = urllib.request.Request(
        lifecycle.endpoint(record) + "/api/tokens", method="POST",
        data=json.dumps({token: {"role": "viewer", "mk_control": False} for token in viewers}).encode(),
        headers={"Authorization": f"Bearer {record['master_token']}", "Content-Type": "application/json"},
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}), NoRedirect())
    with opener.open(request, timeout=2) as response:
        if response.status != 200:
            raise PrivateStreamError("private viewer registration failed")


class ViewerAccess:
    def __init__(self):
        self.token = secrets.token_urlsafe(32)
        self.generation = None
        self.origin = None

    def __enter__(self):
        try:
            with locked_state() as (directory, record):
                # A recorded dead generation is a crash, not an explicit stop
                # (which removes the record). Recover only the streamer, on its
                # original display and port. The lock serializes viewer retries.
                if record is not None and lifecycle.owned_process(record) is None:
                    lifecycle.start(directory, port=record["port"], display=record["display"])
                    record = lifecycle.read_state(directory)
                if lifecycle.owned_process(record) is None or not lifecycle.healthy(record):
                    raise PrivateStreamError("private streamer is not healthy")
                self.generation = (record["pid"], record["created"])
                self.origin = lifecycle.endpoint(record)
                viewers = {token: owner for token, owner in record.get("viewers", {}).items()
                           if lifecycle.owned_process(owner) is not None}
                if len(viewers) >= 8:
                    raise PrivateStreamError("private streamer viewer capacity reached")
                process = psutil.Process()
                viewers[self.token] = {"pid": process.pid, "created": process.create_time()}
                record["viewers"] = viewers
                publish(directory, record)
            return self
        except BaseException:
            self.close()
            raise

    def close(self):
        if self.generation is None:
            return
        with locked_state() as (directory, record):
            if (record is None or (record["pid"], record["created"]) != self.generation
                    or lifecycle.owned_process(record) is None):
                return
            record.setdefault("viewers", {}).pop(self.token, None)
            publish(directory, record)

    def __exit__(self, *_):
        self.close()
