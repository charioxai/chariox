// MP-08/MP-10: FFmpeg exposes structs whose layouts change across library majors.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

test('a missing compiled FFmpeg ABI names the required soname and remedy', { skip: process.platform !== 'linux' || !process.env.CHARIOX_NATIVE_DISPLAY_INCLUDE }, async () => {
  const root = await mkdtemp(path.join(tmpdir(), 'chariox-codec-runtime-'));
  try {
    const source = fileURLToPath(new URL('../apps/kernel/src/display_native/', import.meta.url));
    const probe = path.join(root, 'probe.c'), binary = path.join(root, 'probe');
    await writeFile(probe, `
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
const char *cx_codec_decoder_library(void);
int cx_codec_available(void);
void *__real_dlopen(const char *, int);
void *__wrap_dlopen(const char *name, int flags) {
    return !strncmp(name, "libavcodec.so.", 14) ? NULL : __real_dlopen(name, flags);
}
int main(void) { puts(cx_codec_decoder_library()); return cx_codec_available() ? 1 : 0; }
`);
    const compile = spawnSync('cc', ['-O2', path.join(source, 'codec.c'), path.join(source, 'openh264.c'), probe, '-Wl,--wrap=dlopen', '-lyuv', '-ldl', '-pthread', '-lm', '-o', binary], { encoding: 'utf8' });
    assert.equal(compile.status, 0, compile.stderr);
    const run = spawnSync(binary, [], { encoding: 'utf8' });
    assert.equal(run.status, 0, run.stderr);
    assert(run.stderr.includes(run.stdout.trim()), run.stderr);
    assert.match(run.stderr, /native codec unavailable/);
    assert.match(run.stderr, /ABI/);
    assert.match(run.stderr, /rebuild/);
  } finally { await rm(root, { recursive: true, force: true }); }
});
