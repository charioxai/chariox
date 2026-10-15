// MP-07 / MP-08 / MP-10 / MP-11: supplementary packaging regression checks.
import assert from 'node:assert/strict';
import { mkdir, mkdtemp, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { test } from 'node:test';
import { SLICE_DOCKERFILE, sliceDockerfileInputs, sliceInputRoot, validateSliceBuildContext } from './slice-build-context.mjs';

test('preflight resolves shell/JSON COPY and ADD plus local bind mounts across stages', () => {
  const inputs = sliceDockerfileInputs(`
FROM scratch AS builder
  copy --chmod=0444 ["pins/public.env", "/pins"]
COPY "assets/with space.txt" assets/*.txt \\
  /assets/
ADD assets/archive.tar /archive/
COPY --from=builder --chown=1001:1001 /generated /generated
RUN --mount=type=bind,source=assets/compile.c,target=/compile.c cc /compile.c
RUN --mount=type=bind,from=builder,source=/out,target=/out true
RUN --mount=type=cache,target=/cache true
`);
  assert.deepEqual(inputs, ['pins/public.env', 'assets/with space.txt', 'assets/*.txt', 'assets/archive.tar', 'assets/compile.c']);
  assert.equal(sliceInputRoot('assets/nested/file*.txt'), 'assets/nested');
  assert.equal(sliceInputRoot('pins/public.env'), 'pins/public.env');
  assert.throws(() => sliceInputRoot('*.txt'), /explicit directory/);
  for (const source of ['$INPUT', '../outside', '<<EOF', 'https://example.com/archive', 'assets/**/nested', 'assets/{one,two}'])
    assert.throws(() => sliceDockerfileInputs(`COPY ${source} /`), /unsupported local input/);
  assert.throws(() => sliceDockerfileInputs('COPY --chmod=0555 ["one"]'), /sources and a destination/);
});

test('preflight rejects missing files, unmatched globs, ignored inputs and links', async t => {
  const root = await mkdtemp(join(tmpdir(), 'chariox-slice-inputs-'));
  t.after(() => rm(root, { recursive: true, force: true }));
  await mkdir(dirname(join(root, SLICE_DOCKERFILE)), { recursive: true });
  await mkdir(join(root, 'assets/nested'), { recursive: true });
  const setDockerfile = text => writeFile(join(root, SLICE_DOCKERFILE), `FROM scratch\n${text}\n`);
  await writeFile(join(root, 'assets/file.txt'), 'asset');
  await setDockerfile('COPY --chmod=0444 ["assets/file.txt", "/asset"]\nADD assets/nested /nested\nCOPY assets/*.txt /text/');
  assert.equal((await validateSliceBuildContext(root)).inputs, 3);
  await setDockerfile('COPY assets/*.missing /text/');
  await assert.rejects(validateSliceBuildContext(root), /missing Dockerfile input assets\/\*\.missing/);
  await setDockerfile('RUN --mount=type=bind,source=assets/missing.c,target=/source true');
  await assert.rejects(validateSliceBuildContext(root), /missing Dockerfile input assets\/missing\.c/);
  await setDockerfile('COPY assets/file.txt /asset');
  await rm(join(root, 'assets/file.txt'));
  await assert.rejects(validateSliceBuildContext(root), /missing Dockerfile input assets\/file\.txt/);
  await symlink('/outside', join(root, 'assets/file.txt'));
  await assert.rejects(validateSliceBuildContext(root), /link or special file/);
  await rm(join(root, 'assets/file.txt'));
  await writeFile(join(root, 'assets/file.txt'), 'asset');
  for (const path of ['.dockerignore', `${SLICE_DOCKERFILE}.dockerignore`]) {
    await writeFile(join(root, path), 'assets/file.txt');
    await assert.rejects(validateSliceBuildContext(root), /unsupported ignore file/);
    await rm(join(root, path));
  }
});
