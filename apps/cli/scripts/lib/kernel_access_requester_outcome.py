"""MP-08 / MP-10 / MP-11: distinguish CLI refusal from provider exit status."""

REFUSAL_DIAGNOSTIC = (
    "kernel transport `handle kernel response` failed: "
    "local transport `kernel access` failed: access request refused or expired; "
    "answer the popup in a Chariox terminal"
)


def verify_requester_exit(returncode, diagnostic, *, local_cli):
    if local_cli:
        # The direct CLI propagates the kernel refusal as exit 1. Require its
        # exact public diagnostic; transport/startup errors must still fail.
        lines = diagnostic.splitlines()
        if returncode != 1 or not lines or lines[-1] != REFUSAL_DIAGNOSTIC:
            raise RuntimeError("MP-10 local CLI did not report the expected access refusal")
        return REFUSAL_DIAGNOSTIC
    if returncode != 0:
        raise RuntimeError("MP-10 outside requester exited " + str(returncode))
