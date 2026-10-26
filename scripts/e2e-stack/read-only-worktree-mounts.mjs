// MP-08 / MP-10 / MP-11: read-only source mounts for isolated drill workers.
import { execFileSync } from 'node:child_process';
import { isAbsolute, relative, resolve } from 'node:path';

export function readOnlyWorktreeMounts(worktree) {
  const source = resolve(worktree);
  const common = execFileSync('git', ['-C', source, 'rev-parse', '--path-format=absolute', '--git-common-dir'], {
    encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'],
  }).trim();
  if (!isAbsolute(common)) throw new Error('Git common directory must be absolute');
  const paths = [source];
  const inside = relative(source, common);
  if (inside === '..' || inside.startsWith('../') || isAbsolute(inside)) paths.push(common);
  return paths.flatMap(path => {
    if (/[\r\n,]/.test(path)) throw new Error('Git source path cannot be represented as a Docker bind mount');
    return ['--mount', `type=bind,src=${path},dst=${path},readonly`];
  });
}
