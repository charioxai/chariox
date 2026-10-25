// MP-08 / MP-09 / MP-10 / MP-11: the shared subagent definitions (agents/common.md names `scout`) live under the coordinator's
// HOME, but a lanes worker runs with its account profile's config directory, so each definition is copied there.
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';

const NAME = /^[A-Za-z0-9][A-Za-z0-9_-]{0,60}$/;

// Returns what was written. A definition the source does not have is reported, never skipped silently.
export function syncWorkerAgents({ source, target, names = ['scout'] }) {
  const written = [], missing = [];
  for (const name of names) {
    if (!NAME.test(name)) throw new Error(`invalid agent name ${name}`);
    const from = join(source, `${name}.md`), to = join(target, 'agents', `${name}.md`);
    if (!existsSync(from)) { missing.push(name); continue; }
    const text = readFileSync(from, 'utf8');
    if (existsSync(to) && readFileSync(to, 'utf8') === text) continue;
    mkdirSync(join(target, 'agents'), { recursive: true });
    writeFileSync(to, text, { mode: 0o600 });
    written.push(name);
  }
  return { written, missing };
}
