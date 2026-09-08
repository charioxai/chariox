// Manually admitted unsigned build on a disposable hosted Mac, never a laptop.
import { appendFile, mkdir, mkdtemp, readFile, realpath } from 'node:fs/promises';
import { join } from 'node:path';
import { buildRuntime, checkedOutput, createPlan } from './build-app-runtime.mjs';
import { checkMacBuilder } from './app-runtime-macos-build.mjs';

const profile = JSON.parse(await readFile(new URL('../apps/app-worker/macos-build-profile.json', import.meta.url), 'utf8'));
checkMacBuilder(profile, process.env);
const root = await mkdtemp(join(await realpath(process.env.RUNNER_TEMP), 'chariox-macos-native.'));
await appendFile(process.env.GITHUB_OUTPUT, `scratch=${root}\nartifact_path=${root}/runtime/artifacts\nevidence=${root}/runtime/macos-build-resources.json\n`);
const lock = JSON.parse(await readFile(new URL('../apps/app-worker/runtime.lock.json', import.meta.url), 'utf8'));
const environment = { PATH: '/usr/bin:/bin', LANG: 'C', LC_ALL: 'C', DEVELOPER_DIR: profile.developerDirectory };
const cc = checkedOutput('/usr/bin/xcrun', ['--find', 'clang'], environment);
const cxx = checkedOutput('/usr/bin/xcrun', ['--find', 'clang++'], environment);
const python = await realpath(process.env.NATIVE_PYTHON);
const plan = createPlan({ target: 'darwin-arm64', resourceProfile: 'github-macos', scratch: join(root, 'runtime'),
  jobs: 1, downloadSource: true, cc, cxx, python }, lock);
plan.buildHome = join(root, 'home');
await mkdir(plan.buildHome, { mode: 0o700 });
try {
  const manifest = await buildRuntime(plan, lock);
  console.log(JSON.stringify({ target: manifest.target, signing: manifest.signing, resourceObservation: manifest.resourceObservation }));
} catch (error) {
  console.error(`unsigned macOS native build failed: ${error.message}`);
  process.exitCode = 1;
}
