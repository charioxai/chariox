// Reuse the measured spike entries through the production SDK/compiler path.
import { copyFile, mkdir, symlink, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

const directory = process.argv[2];
const expected = {};
for (const name of ['zod', 'date-fns', 'lodash-es', 'nanoid', 'marked']) {
  const source = join(directory, name);
  await mkdir(source);
  await symlink(resolve('packages/app-sdk-tools/node_modules'), join(source, 'node_modules'), 'dir');
  await copyFile(`spikes/wasm/npm-compat/entries/${name}.js`, join(source, 'entry.mjs'));
  const entry = join(source, 'app.mjs');
  await writeFile(entry, 'import {defineApp} from "@chariox/app-sdk-wasm"; import {run} from "./entry.mjs"; export const handlers=defineApp({tools:{run:async()=>run()}});');
  const result = spawnSync(process.execPath, ['packages/app-sdk-tools/build-js.mjs', entry, join(process.env.CHARIOX_SDK_COMPONENTS, `npm-${name}.wasm`)], { encoding: 'utf8' });
  if (result.status !== 0) throw Error(`${name} build failed: ${result.stderr}`);
  expected[name] = await (await import(pathToFileURL(join(source, 'entry.mjs')).href)).run();
}
await writeFile(join(process.env.CHARIOX_SDK_COMPONENTS, 'npm-expected.json'), JSON.stringify(expected));
