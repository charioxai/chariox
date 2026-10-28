#!/usr/bin/env python3
"""Run actual SDK exports and template natively under ASan/UBSan."""
import os
from pathlib import Path
import subprocess
import tempfile
from build import bindings
from toolchain import SDK


def run():
    with tempfile.TemporaryDirectory(prefix="chariox-c-test-") as temporary:
        scratch = Path(temporary)
        bindings(scratch)
        flags = [
            "-std=c11", "-O1", "-g", "-Wall", "-Wextra", "-Werror",
            "-fsanitize=address,undefined", "-fno-sanitize-recover=all", "-fno-omit-frame-pointer",
            "-I" + str(scratch), "-I" + str(SDK / "include"), "-I" + str(SDK / "tests"),
        ]
        compiler = os.environ.get("CC", "clang")
        runtime = sorted((SDK / "src").glob("*.c"))
        template = SDK / "template/app.c"
        transform = SDK / "tests/fixtures/transform.c"
        sources = runtime + [
            template, transform, SDK / "tests/abi.c", SDK / "tests/alloc.c",
            *sorted((SDK / "tests").glob("test-*.c")),
        ]
        objects = {}
        for source in sources:
            obj = scratch / f"{source.parent.name}-{source.stem}.o"
            instrument = []
            if source in runtime or source in (template, transform):
                instrument = ["-include", str(SDK / "tests/alloc.h")]
            subprocess.run([compiler, *flags, *instrument, "-c", str(source), "-o", str(obj)], check=True)
            objects[source] = obj
        for name, app in [("json", None), ("reader", None), ("yield", None), ("cancel", None), ("task", None), ("review", None), ("template", template), ("transform", transform)]:
            needed = [SDK / f"tests/test-{name}.c", SDK / "tests/alloc.c"]
            needed += [SDK / "src/json.c"] if name == "json" else runtime + [SDK / "tests/abi.c"]
            if app:
                needed.append(app)
            binary = scratch / f"test-{name}"
            subprocess.run([compiler, *flags, *[str(objects[p]) for p in needed], "-lm", "-o", str(binary)], check=True)
            subprocess.run([str(binary)], env={
                **os.environ, "ASAN_OPTIONS": "malloc_fill_byte=190:detect_leaks=1",
                "UBSAN_OPTIONS": "halt_on_error=1",
            }, check=True)


if __name__ == "__main__":
    run()
