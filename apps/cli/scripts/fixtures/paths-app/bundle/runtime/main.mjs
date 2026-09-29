// Path-safety drill (V-SDK-02): SDK atomic writes racing the App's own node:fs
// changes inside its private data. Every tool reports what the App sees
// afterwards; a correct kernel never tears a file, never leaves a staging
// file behind and never writes outside the private data root.
//   rename_race     atomicReplace into a directory the App keeps renaming
//   unicode_names   NFC/NFD and case variants of one name
//   deleted_parent  atomicReplace into a directory the App keeps deleting
//   listing         every entry under private data, with sizes
import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

const settle = (promise) => promise.then(() => 'ok', (error) => error?.code ?? 'failed');
const tally = (values) => values.reduce((counts, value) => ({ ...counts, [value]: (counts[value] ?? 0) + 1 }), {});

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
    for (let round = 0; round < rounds; round += 1) {
      const write = settle(chariox.files.atomicReplace('race/a/f', payload(round)));
      try { renameSync(at('race/a'), at('race/b')); renameSync(at('race/b'), at('race/a')); } catch (error) { outcomes.push(`rename:${error.code}`); }
      outcomes.push(await write);
      if (!existsSync(at('race/a'))) mkdirSync(at('race/a'), { recursive: true });
    }
    const files = walk(at('race'));
    const torn = files.filter((file) => !whole(readFileSync(at(file.path))) && file.path.endsWith('/f'));
    return { rounds, outcomes: tally(outcomes), files, torn: torn.length };
  });

  chariox.tools.register('unicode_names', async () => {
    rmSync(at('names'), { recursive: true, force: true });
    mkdirSync(at('names'));
    const names = { nfc: 'café', nfd: 'café', upper: 'Case', lower: 'case' };
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
    const outcomes = [];
    for (let round = 0; round < rounds; round += 1) {
      mkdirSync(at('gone/x'), { recursive: true });
      const write = settle(chariox.files.atomicReplace('gone/x/f', payload(round)));
      rmSync(at('gone'), { recursive: true, force: true });
      outcomes.push(await write);
    }
    const left = existsSync(at('gone')) ? walk(at('gone')) : [];
    return { rounds, outcomes: tally(outcomes), left };
  });

  chariox.tools.register('listing', async () => ({ entries: walk(root) }));
}
