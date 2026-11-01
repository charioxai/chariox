// MP-08 / MP-10 / MP-11: public Copy through real CLI, TUI approval and hosted WSS.
import assert from 'node:assert/strict';
import { mkdir, writeFile, readFile, lstat, realpath } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { root, out, kernel, binary, origin, relay, sha, record, runtime, enroll, project, selection, copy, launchTarget, launchBrowser, close, release } from './workspace-runtime.mjs';
import { fixture, tree, gitState, text, collision } from './workspace-fixture.mjs';
await mkdir(root, { recursive: false, mode: 0o700 });
await mkdir(out, { recursive: true, mode: 0o700 });
let failure;
try {
    await record('identity', { kernelHead: process.env.CHARIOX_WORKSPACE_KERNEL_HEAD, driverHead: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(), kernelSha256: sha(await readFile(kernel)), tuiSha256: sha(await readFile(binary)), origin, relay });
    await launchBrowser();
    const source = await runtime('source'), target = await runtime('target');
    await enroll(source);
    await enroll(target);
    assert.notEqual(source.status.daemon_id, target.status.daemon_id);
    const paths = await fixture(), before = { git: gitState(paths.primary), primary: await tree(paths.primary), notes: await tree(paths.notes) };
    const id = await project(source, paths);
    const approval = { primary: target.path + '/approval', notes: target.path + '/approval-notes' };
    await mkdir(approval.primary);
    await mkdir(approval.notes);
    await project(target, approval);
    const selected = await selection(source, target, id, paths);
    const outbound = await copy(source, selected, 'outbound');
    assert.equal(outbound.phase, 'completed');
    assert.equal(outbound.receipt.kernelContext.kind, 'empty');
    const materialized = await launchTarget(target, outbound);
    assert.equal(materialized.kind, 'from_source');
    const moved = { primary: materialized.repositories.find(r => r.role === 'primary').workspacePath, notes: materialized.repositories.find(r => r.role === 'supporting').workspacePath };
    assert.ok((await lstat(moved.primary + '/.git')).isDirectory());
    assert.equal(await realpath(moved.primary + '/' + text(moved.primary, ['rev-parse', '--git-common-dir'])), moved.primary + '/.git');
    assert.deepEqual(gitState(moved.primary), before.git);
    assert.deepEqual(await tree(moved.primary), before.primary);
    assert.deepEqual(await tree(moved.notes), before.notes);
    assert.ok((await lstat(paths.primary + '/.git')).isFile());
    await record('outbound-identical', { gitState: before.git, primary: before.primary, notes: before.notes, linkedSourcePreserved: true, selfContainedTarget: true });
    const occupied = await collision(target);
    try {
        const refused = await copy(source, selected, 'divergent');
        await occupied.diagnostics();
        assert.equal(refused.phase, 'failed');
        await occupied.verify();
    }
    finally {
        occupied.close();
    }
    assert.deepEqual(gitState(paths.primary), before.git);
    assert.deepEqual(await tree(paths.primary), before.primary);
    assert.deepEqual(await tree(paths.notes), before.notes);
    await writeFile(moved.primary + '/input', 'latest target work\n');
    await writeFile(moved.notes + '/nested/document', 'latest target notes\n');
    const latest = { git: gitState(moved.primary), primary: await tree(moved.primary), notes: await tree(moved.notes) };
    const targetId = await project(target, moved, materialized.projectId);
    const back = await copy(target, await selection(target, source, targetId, moved), 'return');
    assert.equal(back.phase, 'completed');
    const home = await launchTarget(source, back), homePrimary = home.repositories.find(r => r.role === 'primary').workspacePath, homeNotes = home.repositories.find(r => r.role === 'supporting').workspacePath;
    assert.deepEqual(gitState(homePrimary), latest.git);
    assert.deepEqual(await tree(homePrimary), latest.primary);
    assert.deepEqual(await tree(homeNotes), latest.notes);
    assert.deepEqual(gitState(paths.primary), before.git);
    await record('fresh-return-identical', { gitState: latest.git, originalSourceUnchanged: true, freshDestination: home.destinationRoot });
    await record('summary', { result: 'PASS', realKernels: 2, publicCli: true, sourceTuiReview: true, hostedWss: true, dirtyGit: true, directory: true, divergentTargetUnchanged: true, freshReturn: true });
}
catch (error) {
    failure = error;
    await record('summary', { result: 'FAIL', error: String(error), stack: error.stack });
}
finally {
    await close();
    release();
}
if (failure)
    throw failure;
