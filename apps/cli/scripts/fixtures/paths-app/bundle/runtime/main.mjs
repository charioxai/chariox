// Path-safety drill (V-SDK-02): SDK atomic writes racing the App's own node:fs
// changes inside its private data. Every tool reports what the App sees
// afterwards; a correct kernel never tears a file, never leaves a staging
// file behind and never writes outside the private data root.
//   rename_race     atomicReplace while the App keeps renaming its parent
//   unicode_names   NFC/NFD and case variants of one name
//   deleted_parent  atomicReplace while the App keeps deleting its parent
//   listing         every entry under private data, with sizes
// The mutations run on every event-loop turn until the write settles, so they
// overlap the kernel's staging and publication; `overlapped` counts the rounds
// in which at least one mutation landed while the write was pending.
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const settle = (promise) => promise.then(() => 'ok', (error) => error?.code ?? 'failed');
const tally = (values) => values.reduce((counts, value) => ({ ...counts, [value]: (counts[value] ?? 0) + 1 }), {});

/** Runs `mutate` on each event-loop turn until `write` settles. */
async function racing(write, mutate) {
  let settled = false;
  const outcome = settle(write).then((value) => { settled = true; return value; });
  let mutations = 0;
  const errors = [];
  await new Promise((resolve) => {
    const turn = () => {
      if (settled) { resolve(); return; }
      try { mutate(); mutations += 1; } catch (error) { errors.push(error?.code ?? 'failed'); }
      setImmediate(turn);
    };
    turn();
  });
  return { outcome: await outcome, mutations, errors };
}

export default function register(chariox) {
  const root = chariox.paths.data;
  const at = (path) => join(root, path);
  const walk = (dir) => readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory() ? walk(path) : [{ path: relative(root, path), bytes: statSync(path).size }];
  });
  // A payload is whole only if every byte is its round's marker.
  const payload = (round) => Buffer.alloc(64 * 1024, 65 + (round % 26));
  const whole = (bytes) => bytes.length === 64 * 1024 && bytes.every((b) => b === bytes[0]);

  chariox.tools.register('rename_race', async ({ rounds = 200 }) => {
    rmSync(at('race'), { recursive: true, force: true });
    mkdirSync(at('race/a'), { recursive: true });
    const outcomes = [];
    const errors = [];
    let overlapped = 0;
    let mutations = 0;
    for (let round = 0; round < rounds; round += 1) {
      const result = await racing(chariox.files.atomicReplace('race/a/f', payload(round)), () => {
        if (existsSync(at('race/a'))) renameSync(at('race/a'), at('race/b'));
        else renameSync(at('race/b'), at('race/a'));
      });
      outcomes.push(result.outcome);
      errors.push(...result.errors);
      mutations += result.mutations;
      if (result.mutations > 0) overlapped += 1;
      if (!existsSync(at('race/a'))) renameSync(at('race/b'), at('race/a'));
    }
    const files = walk(at('race'));
    const torn = files.filter((file) => file.path.endsWith('/f') && !whole(readFileSync(at(file.path))));
    return { rounds, overlapped, mutations, outcomes: tally(outcomes), mutationErrors: tally(errors), files, torn: torn.length };
  });

  chariox.tools.register('unicode_names', async () => {
    rmSync(at('names'), { recursive: true, force: true });
    mkdirSync(at('names'));
    const names = { nfc: 'caf\u00e9', nfd: 'cafe\u0301', upper: 'Case', lower: 'case' };
    const results = {};
    for (const [label, name] of Object.entries(names)) {
      results[label] = await settle(chariox.files.atomicReplace(`names/${name}`, label));
    }
    const entries = readdirSync(at('names')).map((name) => ({
      name: [...name].map((c) => c.codePointAt(0).toString(16)).join(' '),
      contents: readFileSync(at(join('names', name)), 'utf8'),
    }));
    return { writes: results, entries };
  });

  chariox.tools.register('deleted_parent', async ({ rounds = 200 }) => {
    rmSync(at('gone'), { recursive: true, force: true });
    const outcomes = [];
    const errors = [];
    let overlapped = 0;
    let mutations = 0;
    for (let round = 0; round < rounds; round += 1) {
      mkdirSync(at('gone/x'), { recursive: true });
      const result = await racing(chariox.files.atomicReplace('gone/x/f', payload(round)), () => {
        if (existsSync(at('gone'))) rmSync(at('gone'), { recursive: true, force: true });
        else mkdirSync(at('gone/x'), { recursive: true });
      });
      outcomes.push(result.outcome);
      errors.push(...result.errors);
      mutations += result.mutations;
      if (result.mutations > 0) overlapped += 1;
    }
    const left = existsSync(at('gone')) ? walk(at('gone')) : [];
    const torn = left.filter((file) => file.path.endsWith('/f') && !whole(readFileSync(at(file.path))));
    return { rounds, overlapped, mutations, outcomes: tally(outcomes), mutationErrors: tally(errors), left, torn: torn.length };
  });

  chariox.tools.register('listing', async () => ({ entries: walk(root) }));
}
