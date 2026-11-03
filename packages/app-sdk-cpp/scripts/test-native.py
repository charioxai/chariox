#!/usr/bin/env python3
"""Call the C++ SDK's real exports under the C SDK's ASan/UBSan ABI simulator."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

SDK = Path(__file__).resolve().parents[1]
C_SDK = SDK.parent / "app-sdk-c"
sys.path.insert(0, str(C_SDK / "scripts"))
from build import bindings


def run(cases):
    with tempfile.TemporaryDirectory(prefix="chariox-cpp-test-") as temporary:
        scratch = Path(temporary)
        bindings(scratch)
        flags = [
            "-O1", "-g", "-Wall", "-Wextra", "-Werror",
            "-fsanitize=address,undefined", "-fno-sanitize-recover=all", "-fno-omit-frame-pointer",
            "-I" + str(scratch), "-I" + str(C_SDK / "include"),
            "-I" + str(C_SDK / "tests"), "-I" + str(SDK / "include"),
        ]
        cc, cpp = os.environ.get("CC", "clang"), os.environ.get("CXX", "clang++")
        runtime = sorted((C_SDK / "src").glob("*.c"))
        common = runtime + [C_SDK / "tests/abi.c", C_SDK / "tests/alloc.c"]
        objects = []
        for source in common:
            obj = scratch / (source.stem + ".o")
            instrument = ["-include", str(C_SDK / "tests/alloc.h")] if source in runtime else []
            subprocess.run([cc, "-std=c11", *flags, *instrument, "-c", str(source), "-o", str(obj)], check=True)
            objects.append(obj)
        for name in ("call", "template"):
            source = SDK / ("tests/test-call.cpp" if name == "call" else "template/app.cpp")
            app = scratch / (name + ".o")
            subprocess.run([cpp, "-std=c++20", "-fno-exceptions", *flags,
                            "-include", str(SDK / "tests/alloc.hpp"),
                            "-c", str(source), "-o", str(app)], check=True)
            needed = [*objects, app]
            if name == "template":
                suite = scratch / "test-template.o"
                subprocess.run([cc, "-std=c11", *flags, "-c", str(C_SDK / "tests/test-template.c"), "-o", str(suite)], check=True)
                needed.append(suite)
            binary = scratch / ("test-" + name)
            subprocess.run([cpp, *flags, *map(str, needed), "-lm", "-o", str(binary)], check=True)
            subprocess.run([str(binary), *(cases if name == "call" else [])], env={
                **os.environ, "ASAN_OPTIONS": "malloc_fill_byte=190:detect_leaks=1",
                "UBSAN_OPTIONS": "halt_on_error=1",
            }, check=True)
        # Every App translation unit must refuse exceptions at compile time.
        probe = scratch / "probe.cpp"
        probe.write_text('#include "chariox/app.hpp"\n')
        result = subprocess.run([cpp, "-std=c++20", "-fexceptions", *flags,
                                 "-fsyntax-only", str(probe)], capture_output=True, text=True)
        if result.returncode == 0 or "requires -fno-exceptions" not in result.stderr:
            raise AssertionError("App headers must reject exception-enabled builds")
        print("exceptions: exception-enabled App compilation rejected", flush=True)


if __name__ == "__main__":
    run(sys.argv[1:])
