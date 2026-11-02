"""MP-08 / MP-09 / MP-10 / MP-11: private owner setup for the real DNS drill."""

import argparse
import json
import os
from pathlib import Path
from secrets import token_hex
import sys
import urllib.error
import urllib.request

import dynv6


def mail_api(path, body=None):
    request = urllib.request.Request(
        "https://api.mail.tm" + path,
        data=json.dumps(body).encode() if body else None,
        headers={"Content-Type": "application/json"},
    )
    with urllib.request.urlopen(request, timeout=30) as response:
        return json.load(response)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-dir", required=True, type=Path)
    directory = parser.parse_args().state_dir
    if not directory.is_absolute() or directory.resolve().is_relative_to(Path.cwd()):
        raise RuntimeError(
            "use an absolute private lane directory outside the workspace"
        )
    directory.mkdir(parents=True, exist_ok=True, mode=0o700)
    if (directory / "mail.json").exists() or (directory / "account.json").exists():
        raise RuntimeError(
            "previous setup exists; reconcile its cleanup before another signup"
        )

    domain = next(
        item["domain"]
        for item in mail_api("/domains")["hydra:member"]
        if item["isActive"]
    )
    auth = {
        "address": "chariox-vflag-" + token_hex(8) + "@" + domain,
        "password": token_hex(32),
    }
    mail_path = directory / "mail.json"
    descriptor = os.open(mail_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "w") as output:
        json.dump(auth, output)
        output.flush()
        os.fsync(output.fileno())
    account = mail_api("/accounts", auth)
    auth["id"] = account["id"]
    auth["token"] = mail_api(
        "/token", {key: auth[key] for key in ["address", "password"]}
    )["token"]
    with mail_path.open("w") as output:
        json.dump(auth, output)
        output.flush()
        os.fsync(output.fileno())

    dynv6.STATE = directory / "account.json"
    result = dynv6.signup({"email": auth["address"], "password": token_hex(24)})
    print(
        json.dumps(
            {
                "MP": "MP-08 / MP-09 / MP-10 / MP-11",
                "signup": result,
                "private_cleanup_access_retained": True,
            }
        )
    )
    if not result["registered"]:
        raise RuntimeError(
            "signup was not confirmed; retain private setup and reconcile cleanup"
        )


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(
            json.dumps(
                {
                    "error": type(error).__name__,
                    "http_status": (
                        error.code
                        if isinstance(error, urllib.error.HTTPError)
                        else None
                    ),
                    "message": (
                        str(error)
                        if type(error) is RuntimeError
                        else "private setup failed"
                    ),
                    "next_action": "retain private setup and reconcile cleanup",
                }
            ),
            file=sys.stderr,
        )
        sys.exit(1)
