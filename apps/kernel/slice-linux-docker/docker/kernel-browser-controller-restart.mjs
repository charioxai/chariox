// Row 548 (2026-11-24): a browser that fails to launch three times in a row is restarted with its controller process, as a kernel
// hot reload does: the next request starts a clean controller that restores the Tabs from their snapshot. At most
// MAX_RESTARTS within WINDOW_MS, so a host that cannot run Chrome at all is not restarted in a loop.
import { readFile, writeFile, rename } from 'node:fs/promises';
import path from 'node:path';
export const LAUNCH_FAILURES_BEFORE_RESTART = 3, MAX_RESTARTS = 3, WINDOW_MS = 600_000;
export const mayRestart = (history, now) => history.filter(at => Number.isFinite(at) && now - at < WINDOW_MS).length < MAX_RESTARTS;
export async function recordRestart(root, now = Date.now()) {
  const file = path.join(root, 'controller-restarts.json');
  let history = [];
  try { history = JSON.parse(await readFile(file, 'utf8')); } catch {}
  if (!Array.isArray(history)) history = [];
  history = history.filter(at => Number.isFinite(at) && now - at < WINDOW_MS);
  if (!mayRestart(history, now)) return false;
  await writeFile(`${file}.new`, JSON.stringify([...history, now]), { mode: 0o600 });
  await rename(`${file}.new`, file);
  return true;
}
