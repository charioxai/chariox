"""Compile the production kernel library, then time only the release regression."""
import json
import subprocess
import tempfile
import time

TEST_NAME = "tests::visible_session_project_assignment_survives_release_builds"
TARGET = "chariox_kernel_release_tests"


def phase(name, command, **kwargs):
    started = time.monotonic()
    print(f"Release regression {name} started", flush=True)
    result = subprocess.run(["/usr/bin/time", "-v", *command], **kwargs)
    print(f"Release regression {name} finished: status={result.returncode} "
          f"elapsed={time.monotonic()-started:.2f}s", flush=True)
    result.check_returncode()


def main():
    # The kernel is a normal dependency, so its dev-only App fixtures stay out.
    # Keep kernel release optimization and disabled debug assertions unchanged.
    command = ["cargo", "test", "--release", "--locked", "-p",
               "chariox-kernel-release-tests", "--lib", "--no-run", "--message-format=json-render-diagnostics",
               "--config=profile.release.package.chariox-kernel.codegen-units=256"]
    with tempfile.TemporaryFile(mode="w+") as output:
        phase("compile", command, stdout=output)
        output.seek(0)
        executables = []
        for line in output:
            message = json.loads(line)
            if message.get("reason") == "compiler-message":
                print(message["message"].get("rendered", ""), end="", flush=True)
            if (message.get("reason") == "compiler-artifact"
                    and message["target"]["name"] == TARGET
                    and message["profile"]["test"] and message.get("executable")):
                executables.append(message["executable"])
    if len(executables) != 1:
        raise ValueError(f"Expected one release regression executable, found {len(executables)}")
    # No Cargo lock, freshness check or compilation is inside the test deadline.
    phase("test", ["timeout", "--signal=KILL", "60", executables[0],
                   "--exact", TEST_NAME, "--nocapture"])


if __name__ == "__main__":
    main()
