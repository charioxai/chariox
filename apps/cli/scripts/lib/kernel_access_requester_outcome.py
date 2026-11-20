"""MP-08 / MP-10 / MP-11: distinguish CLI denial from provider exit status."""

import json

REFUSAL_DIAGNOSTIC = (
    "The owner denied kernel access. Run `chariox access request --no-wait` "
    "if you still need access."
)


def verify_requester_exit(returncode, diagnostic, *, local_cli):
    if local_cli:
        lines = diagnostic.splitlines()
        try:
            error = json.loads(lines[-1])["error"]
        except (IndexError, KeyError, TypeError, ValueError) as exc:
            raise RuntimeError("MP-10 local CLI did not report the expected access denial") from exc
        if returncode != 3 or error.get("code") != "kernel_access_request_denied" or error.get("message") != REFUSAL_DIAGNOSTIC:
            raise RuntimeError("MP-10 local CLI did not report the expected access denial")
        return REFUSAL_DIAGNOSTIC
    if returncode != 0:
        raise RuntimeError("MP-10 outside requester exited " + str(returncode))
