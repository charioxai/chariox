// MP-08/MP-10/MP-11: optional display libraries must never become ELF startup dependencies.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

test('native capture starts without X11 libraries and refuses unavailable capture clearly', { skip: process.platform !== 'linux' || !process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'chariox-native-runtime-'));
  try {
    const probe = path.join(root, 'probe.c'), binary = path.join(root, 'probe');
    await writeFile(probe, `
#include <dlfcn.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
int cx_capture_inventory(unsigned long, int64_t *, int);
void *__real_dlopen(const char *, int);
void *__wrap_dlopen(const char *name, int flags) {
    const char *missing = getenv("MISSING_LIBRARY");
    return missing && !strcmp(name, missing) ? NULL : __real_dlopen(name, flags);
}
int main(void) { int64_t rows[32*13]; return cx_capture_inventory(2, rows, 32) == -1 ? 0 : 1; }
`);
    const source = fileURLToPath(new URL('../apps/kernel/src/display_native/capture.c', import.meta.url));
    const compile = spawnSync('cc', ['-O2', source, probe, '-Wl,--wrap=dlopen', '-ldl', '-pthread', '-o', binary], { encoding: 'utf8' });
    assert.equal(compile.status, 0, compile.stderr);
    const elf = spawnSync('readelf', ['-d', binary], { encoding: 'utf8' });
    assert.equal(elf.status, 0, elf.stderr);
    assert.doesNotMatch(elf.stdout, /NEEDED.*lib(?:X11|Xext|Xdamage|Xcomposite|Xtst)\./);
    for (const library of ['libX11.so.6', 'libXext.so.6', 'libXdamage.so.1', 'libXcomposite.so.1', 'libXtst.so.6']) {
      const run = spawnSync(binary, [], { encoding: 'utf8', env: { ...process.env, MISSING_LIBRARY: library, DISPLAY: '' } });
      assert.equal(run.status, 0, run.stderr);
      assert.match(run.stderr, /native display unavailable/);
      assert(run.stderr.includes(library), run.stderr);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});
