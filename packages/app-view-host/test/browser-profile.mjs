import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// Incognito Chromium suppresses explicit preconnect and DNS-prefetch, making
// their positive controls vacuous. Use a fresh ordinary profile, then remove it.
export async function browserProfile(chromium, options) {
  if (Buffer.byteLength(tmpdir()) > 60) throw new Error('Use a TMPDIR shorter than 61 bytes for Chromium Unix sockets');
  const directory = await mkdtemp(join(tmpdir(), 'av1-browser-'));
  let context;
  try { context = await chromium.launchPersistentContext(directory, options); }
  catch (error) { await rm(directory, { recursive: true }); throw error; }
  return { context, async close() { try { await context.close(); } finally { await rm(directory, { recursive: true }); } } };
}
