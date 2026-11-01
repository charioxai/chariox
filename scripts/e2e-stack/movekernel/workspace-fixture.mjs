// MP-08 / MP-10 / MP-11: dirty workspace fixtures and immutable-state comparisons.
import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { watch } from 'node:fs';
import { mkdir, writeFile, readFile, readdir, lstat, chmod, rename, rm } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { root, sha, record } from './workspace-runtime.mjs';
const git = (path, args) => execFileSync('git', ['-C', path, ...args], { env: { ...process.env, GIT_CONFIG_GLOBAL: '/dev/null', GIT_CONFIG_NOSYSTEM: '1' } });
const text = (path, args) => git(path, args).toString().trim();
function gitState(path) {
    return [['rev-parse', 'HEAD'], ['symbolic-ref', '--short', 'HEAD'], ['status', '--porcelain=v1', '-z', '--untracked-files=all'], ['diff', '--binary', '--cached'], ['diff', '--binary'], ['ls-files', '--stage', '-z']].map(args => sha(git(path, args)));
}
async function tree(path) {
    const result = [];
    async function walk(relative) {
        for (const name of (await readdir(path + '/' + relative)).sort()) {
            if (name === '.git')
                continue;
            const item = relative ? relative + '/' + name : name, info = await lstat(path + '/' + item);
            assert.ok(!info.isSymbolicLink());
            result.push({ path: item, kind: info.isDirectory() ? 'directory' : 'file', executable: (info.mode & 0o111) !== 0, ...(info.isFile() ? { sha256: sha(await readFile(path + '/' + item)), size: info.size } : {}) });
            if (info.isDirectory())
                await walk(item);
        }
    }
    await walk('');
    return result;
}
async function fixture() {
    const main = root + '/main', primary = root + '/linked', notes = root + '/notes';
    await mkdir(main);
    git(main, ['init', '-b', 'main']);
    git(main, ['config', 'user.name', 'Workspace drill']);
    git(main, ['config', 'user.email', 'workspace@invalid']);
    await writeFile(main + '/input', Buffer.from('base\0bytes\n'));
    await writeFile(main + '/removed', 'remove me\n');
    git(main, ['add', '.']);
    git(main, ['commit', '-m', 'Initial workspace']);
    git(main, ['worktree', 'add', '-b', 'move-workspace', primary]);
    await writeFile(primary + '/input', Buffer.from('index\0\xff\n', 'latin1'));
    await chmod(primary + '/input', 0o755);
    git(primary, ['add', 'input']);
    await writeFile(primary + '/input', Buffer.from('working\0\xfe\n', 'latin1'));
    await chmod(primary + '/input', 0o644);
    await rm(primary + '/removed');
    git(primary, ['add', 'removed']);
    await writeFile(primary + '/untracked', randomBytes(8 * 1024 * 1024));
    await mkdir(notes + '/nested/empty', { recursive: true });
    await writeFile(notes + '/nested/document', Buffer.from('notes\0\x81\n', 'latin1'));
    await writeFile(notes + '/run.sh', '#!/bin/sh\nprintf notes\n', { mode: 0o755 });
    await mkdir(notes + '/private-data');
    await writeFile(notes + '/.gitignore', 'private-data/\n');
    await writeFile(notes + '/.charioxignore', '!**/needed.txt\n!private*/explicit.txt\n');
    await writeFile(notes + '/private-data/needed.txt', 'globstar-selected notes\n');
    await writeFile(notes + '/private-data/explicit.txt', 'component-glob-selected notes\n');
    return { primary, notes };
}
async function collision(target) {
    const store = target.path + '/home/managed-context-transfers';
    await mkdir(store, { recursive: true, mode: 0o700 });
    const existing = new Set(await readdir(store));
    const prepared = root + '/divergent';
    await mkdir(prepared + '/linked', { recursive: true });
    git(prepared + '/linked', ['init', '-b', 'divergent']);
    git(prepared + '/linked', ['config', 'user.name', 'Workspace drill']);
    git(prepared + '/linked', ['config', 'user.email', 'workspace@invalid']);
    await writeFile(prepared + '/linked/marker', 'Keep divergent target\n');
    git(prepared + '/linked', ['add', '.']);
    git(prepared + '/linked', ['commit', '-m', 'Divergent target']);
    await writeFile(prepared + '/linked/marker', 'Keep dirty divergent target\n');
    const before = await tree(prepared), state = gitState(prepared + '/linked'), identity = await lstat(prepared);
    const parent = target.env.HOME + '/chariox-contexts';
    await mkdir(parent, { recursive: true, mode: 0o700 });
    let destination, failed;
    const observed = new Set();
    await record('collision-armed', { store, existing: [...existing] });
    const observer = watch(store, (_event, name) => {
        if (name?.endsWith('.archive'))
            observed.add(name);
        void scan();
    });
    async function scan() {
        try {
            for (const name of await readdir(store)) {
                if (!existing.has(name) && /^ctx_[a-zA-Z0-9_-]+\.archive$/.test(name) && !destination) {
                    destination = parent + '/' + name.slice(0, -8);
                    await rename(prepared, destination);
                }
            }
        }
        catch (error) {
            failed = error;
        }
    }
    return { async diagnostics() {
            await record('collision-observed', { store, observed: [...observed], destination, error: failed ? String(failed) : null });
        }, async verify() {
            observer.close();
            if (failed)
                throw failed;
            assert.ok(destination, 'Must occupy the real destination before target import');
            assert.equal((await lstat(destination)).ino, identity.ino);
            assert.deepEqual(await tree(destination), before);
            assert.deepEqual(gitState(destination + '/linked'), state);
            await record('divergent-target-unchanged', { destination, tree: before, gitState: state });
        }, close() {
            observer.close();
        } };
}
export { fixture, tree, gitState, text, collision };
