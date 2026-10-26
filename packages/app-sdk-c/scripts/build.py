#!/usr/bin/env python3
"""Build an R1 App component from C source with the pinned callback ABI."""
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile
from toolchain import SDK, selected_root, tools


def bindings(directory):
    generator, _ = tools(selected_root())
    subprocess.run([
        str(generator), "c", str(SDK.parent / "app-wit" / "wit"),
        "--world", "app", "--out-dir", str(directory),
    ], check=True)


def build(output, source):
    _, compiler = tools(selected_root())
    with tempfile.TemporaryDirectory(prefix="chariox-c-") as temporary:
        generated = Path(temporary)
        bindings(generated)
        subprocess.run([
            str(compiler), "-std=c11", "-O2", "-Wall", "-Wextra", "-Werror",
            "-Wno-unused-parameter", "-Wno-unused-function", "-mexec-model=reactor",
            "-I" + str(generated), "-I" + str(SDK / "include"), str(source),
            *map(str, sorted((SDK / "src").glob("*.c"))), str(generated / "app.c"),
            str(generated / "app_component_type.o"), "-lm", "-o", str(generated / "app.wasm"),
        ], check=True)
        # A failed compile never replaces an existing successful component.
        output.write_bytes((generated / "app.wasm").read_bytes())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("source", type=Path, nargs="?", default=SDK / "template" / "app.c")
    args = parser.parse_args()
    try:
        build(args.output.resolve(), args.source.resolve())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
