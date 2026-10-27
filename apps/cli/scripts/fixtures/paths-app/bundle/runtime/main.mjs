// Path-safety drill (V-SDK-02): SDK atomic writes racing the App's own node:fs
// changes inside its private data. After every round the tools check what the
// App sees; a correct kernel never tears or substitutes a file, never leaves a
// staging file behind and never writes outside the private data root.
//   rename_race     atomicReplace while the App keeps renaming its parent
//   unicode_names   NFC/NFD and case variants of one name
//   deleted_parent  atomicReplace while the App keeps deleting its parent
//   listing         every entry under private data, with sizes
// The mutations run on every event-loop turn until the write settles, so they
// overlap the kernel's staging and publication; `overlapped` counts the rounds
// in which at least one mutation landed while the write was pending, and
// `fewestMutations` shows how thin the thinnest overlap was.
import { existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, renameSync, rmSync } from 'node:fs';
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
    // Start on the next turn: a mutation in this tick runs before the request
    // frame leaves the process, so it would not overlap the kernel's work.
    setImmediate(turn);
  });
  return { outcome: await outcome, mutations, errors };
}

export default function register(chariox) {
  const root = chariox.paths.data;
  const at = (path) => join(root, path);
  // An unreadable directory (ext4's root-owned lost+found) or entry is reported,
  // not fatal; a caller must treat such an entry as an incomplete listing.
  const walk = (dir) => {
    let entries;
    try { entries = readdirSync(dir, { withFileTypes: true }); } catch (error) {
      return [{ path: relative(root, dir), error: error?.code ?? 'failed' }];
    }
    return entries.flatMap((entry) => {
      const path = join(dir, entry.name);
      if (entry.isDirectory()) return walk(path);
      // lstat: a link is listed, not followed; an entry gone since readdir is reported.
      try { return [{ path: relative(root, path), bytes: lstatSync(path).size }]; } catch (error) {
        return [{ path: relative(root, path), error: error?.code ?? 'failed' }];
      }
    });
  };
  // Round r writes 64 KiB of one marker byte. A surviving file is intact only
  // if it is exactly the payload of the round expected to be visible.
  const marker = (round) => 65 + (round % 26);
  const payload = (round) => Buffer.alloc(64 * 1024, marker(round));
  const intact = (bytes, round) => bytes.length === 64 * 1024 && bytes.every((b) => b === marker(round));
  const describe = (bytes) => ({ bytes: bytes.length, first: bytes[0] ?? null, uniform: bytes.every((b) => b === bytes[0]) });
  // Checked after every settled round, before the next write or delete can hide it.
  const MAX_REPORTED = 20;
  const note = (list, entry) => { if (list.length < MAX_REPORTED) list.push(entry); };

  chariox.tools.register('rename_race', async ({ rounds = 200 }) => {
    rmSync(at('race'), { recursive: true, force: true });
    mkdirSync(at('race/a'), { recursive: true });
    const outcomes = [];
    const errors = [];
    let overlapped = 0;
    let mutations = 0;
    let fewestMutations = Infinity;
    // Renaming the directory moves its file, so race/a/f must always hold the
    // last round whose write succeeded.
    let lastOk = null;
    let integrityFailures = 0;
    let leftoverRounds = 0;
    const failures = [];
    const leftovers = [];
    for (let round = 0; round < rounds; round += 1) {
      const result = await racing(chariox.files.atomicReplace('race/a/f', payload(round)), () => {
        if (existsSync(at('race/a'))) renameSync(at('race/a'), at('race/b'));
        else renameSync(at('race/b'), at('race/a'));
      });
      outcomes.push(result.outcome);
      errors.push(...result.errors);
      mutations += result.mutations;
      fewestMutations = Math.min(fewestMutations, result.mutations);
      if (result.mutations > 0) overlapped += 1;
      if (!existsSync(at('race/a'))) renameSync(at('race/b'), at('race/a'));
      if (result.outcome === 'ok') lastOk = round;
      const extra = readdirSync(at('race/a')).filter((name) => name !== 'f');
      if (extra.length || readdirSync(at('race')).length !== 1) {
        leftoverRounds += 1;
        note(leftovers, { round, entries: walk(at('race')).map((file) => file.path) });
      }
      const file = at('race/a/f');
      const bytes = existsSync(file) ? readFileSync(file) : null;
      if (lastOk === null ? bytes !== null : bytes === null || !intact(bytes, lastOk)) {
        integrityFailures += 1;
        note(failures, { round, outcome: result.outcome, expectedRound: lastOk, found: bytes && describe(bytes) });
      }
    }
    const files = walk(at('race'));
    return { rounds, overlapped, mutations, fewestMutations, outcomes: tally(outcomes), mutationErrors: tally(errors),
      integrityFailures, failures, leftoverRounds, leftovers, files };
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
    let fewestMutations = Infinity;
    // A delete can remove a published file, but nothing else writes it: a
    // surviving gone/x/f after a successful round must be that round's payload,
    // and after a refused round an earlier round's, whole.
    let integrityFailures = 0;
    let leftoverRounds = 0;
    let survived = 0;
    const failures = [];
    const leftovers = [];
    for (let round = 0; round < rounds; round += 1) {
      mkdirSync(at('gone/x'), { recursive: true });
      const result = await racing(chariox.files.atomicReplace('gone/x/f', payload(round)), () => {
        if (existsSync(at('gone'))) rmSync(at('gone'), { recursive: true, force: true });
        else mkdirSync(at('gone/x'), { recursive: true });
      });
      outcomes.push(result.outcome);
      errors.push(...result.errors);
      mutations += result.mutations;
      fewestMutations = Math.min(fewestMutations, result.mutations);
      if (result.mutations > 0) overlapped += 1;
      const entries = existsSync(at('gone')) ? walk(at('gone')) : [];
      if (entries.some((file) => file.path !== join('gone', 'x', 'f'))) {
        leftoverRounds += 1;
        note(leftovers, { round, entries: entries.map((file) => file.path) });
      }
      const file = at('gone/x/f');
      if (!existsSync(file)) continue;
      survived += 1;
      const bytes = readFileSync(file);
      const ok = result.outcome === 'ok'
        ? intact(bytes, round)
        : Array.from({ length: Math.min(round, 26) }, (_, back) => round - 1 - back).some((earlier) => intact(bytes, earlier));
      if (!ok) {
        integrityFailures += 1;
        note(failures, { round, outcome: result.outcome, found: describe(bytes) });
      }
    }
    const left = existsSync(at('gone')) ? walk(at('gone')) : [];
    return { rounds, overlapped, mutations, fewestMutations, outcomes: tally(outcomes), mutationErrors: tally(errors),
      survived, integrityFailures, failures, leftoverRounds, leftovers, left };
  });

  chariox.tools.register('listing', async () => ({ entries: walk(root) }));
}
