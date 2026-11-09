// MP-08/MP-09/MP-10/MP-11: a new file inside the Room workspace. The same containment the upload path applies to
// files it reads: no '..', no path outside the workspace, no symlink on the way or at the end.
import path from 'node:path';
import { randomBytes } from 'node:crypto';
import { lstat, mkdir, realpath, rename, rm, writeFile } from 'node:fs/promises';
import { AgentToolRefusal, UserDomainRefusal } from './kernel-browser-refusal.mjs';

const within = (root, file) => { const relative = path.relative(root, file); return relative !== '' && !path.isAbsolute(relative) && relative !== '..' && !relative.startsWith('..' + path.sep); };

/** Checks the path now; `commit(bytes)` writes it later and returns the workspace-relative path. */
export async function planWorkspaceFile(roots, candidate) {
  if (!Array.isArray(roots) || roots.length !== 1) throw new UserDomainRefusal('not_granted');
  let root;
  try { root = await realpath(roots[0]); }
  catch { throw new AgentToolRefusal('workspace_unavailable'); }
  if (typeof candidate !== 'string' || !candidate || Buffer.byteLength(candidate) > 4096 || candidate.includes('\0')) throw new AgentToolRefusal('save_path_invalid');
  if (candidate.split(/[\\/]/).includes('..')) throw new UserDomainRefusal('not_granted');
  const target = path.resolve(root, candidate);
  if (!within(root, target)) throw new UserDomainRefusal('not_granted');
  // The deepest directory that exists must itself be inside the workspace.
  let probe = path.dirname(target), real;
  for (;;) {
    try { real = await realpath(probe); break; }
    catch (error) {
      if (error?.code !== 'ENOENT') throw new AgentToolRefusal('save_path_invalid');
      probe = path.dirname(probe);
      if (probe.length < root.length) throw new UserDomainRefusal('not_granted');
    }
  }
  if (real !== root && !within(root, real)) throw new UserDomainRefusal('not_granted');
  const existing = await lstat(target).catch(() => null);
  if (existing?.isSymbolicLink()) throw new UserDomainRefusal('not_granted');
  if (existing && !existing.isFile()) throw new AgentToolRefusal('save_path_invalid');
  return {
    async commit(bytes) {
      const directory = path.dirname(target);
      await mkdir(directory, { recursive: true });
      const realDirectory = await realpath(directory);
      if (realDirectory !== root && !within(root, realDirectory)) throw new UserDomainRefusal('not_granted');
      const final = path.join(realDirectory, path.basename(target));
      if ((await lstat(final).catch(() => null))?.isSymbolicLink()) throw new UserDomainRefusal('not_granted');
      const partial = path.join(realDirectory, `.save-${randomBytes(6).toString('hex')}.part`);
      try { await writeFile(partial, bytes, { flag: 'wx', mode: 0o644 }); await rename(partial, final); }
      catch (error) { await rm(partial, { force: true }); throw error; }
      return path.relative(root, final).split(path.sep).join('/');
    },
  };
}
