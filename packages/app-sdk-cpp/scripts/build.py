#!/usr/bin/env python3
"""Build a C++20 App over the pinned C SDK, with exceptions disabled."""
import argparse
from pathlib import Path
import subprocess
import sys
import tempfile

SDK = Path(__file__).resolve().parents[1]
C_SDK = SDK.parent / "app-sdk-c"
sys.path.insert(0, str(C_SDK / "scripts"))
from build import bindings
from toolchain import selected_root, tools


def build(output, source):
    _, compiler = tools(selected_root())
    cpp = compiler.with_name(compiler.name + "++")
    if not cpp.is_file():
        raise ValueError("pinned wasi-sdk lacks clang++; reinstall the C SDK toolchain")
    with tempfile.TemporaryDirectory(prefix="chariox-cpp-") as temporary:
        scratch = Path(temporary)
        bindings(scratch)
        flags = [
            "-O2", "-Wall", "-Wextra", "-Werror", "-Wno-unused-parameter",
            "-Wno-unused-function", "-I" + str(scratch),
            "-I" + str(C_SDK / "include"), "-I" + str(SDK / "include"),
        ]
        objects = []
        for c_source in [*sorted((C_SDK / "src").glob("*.c")), scratch / "app.c"]:
            obj = scratch / (c_source.stem + ".o")
            subprocess.run([str(compiler), "-std=c11", *flags, "-c", str(c_source), "-o", str(obj)], check=True)
            objects.append(obj)
        app = scratch / "application.o"
        subprocess.run([str(cpp), "-std=c++20", "-fno-exceptions", *flags,
                        "-c", str(source), "-o", str(app)], check=True)
        component = scratch / "app.wasm"
        subprocess.run([str(cpp), "-fno-exceptions", "-mexec-model=reactor", str(app),
                        *map(str, objects), str(scratch / "app_component_type.o"),
                        "-lm", "-o", str(component)], check=True)
        # Do not replace a successful component after a failed compile/link.
        output.write_bytes(component.read_bytes())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("source", type=Path, nargs="?", default=SDK / "template" / "app.cpp")
    args = parser.parse_args()
    try:
        build(args.output.resolve(), args.source.resolve())
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        sys.exit(str(error))
