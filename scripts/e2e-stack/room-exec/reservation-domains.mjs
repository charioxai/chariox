// MP-08 / MP-09 / MP-10 / MP-11: one reservation per binary domain and resource.
import assert from 'node:assert/strict'
import {spawn, execFileSync} from 'node:child_process'
import {createHash} from 'node:crypto'
import {mkdir, readdir, rmdir, rm, stat, statfs, writeFile} from 'node:fs/promises'
import path from 'node:path'
import {Fixture, until} from './fixture.mjs'

const args = process.argv.slice(2)
const value = flag => {const at = args.indexOf(flag); assert.ok(at >= 0 && args[at + 1], `missing ${flag}`); return path.resolve(args[at + 1])}
const home = value('--home'), evidence = value('--evidence'), mib = 1048576, gib = 1024 * mib
const disk = args.includes('--disk'), cgroup = `/sys/fs/cgroup/chariox-p1runon-domains-${process.pid}`
const image = `${home}-volume.img`, mountpoint = path.join(home, 'A/work')
const key = (argv, cwd) => {
  const hash = createHash('sha256')
  for (const argument of argv) {hash.update(argument.replace(/[a-fA-F0-9]+/g, run => run.length >= 6 || /^\d+$/.test(run) ? '#' : run)); hash.update(Buffer.from([0]))}
  hash.update(cwd); return hash.digest('hex').slice(0, 32)
}
const child = ['/usr/bin/python3', '-c', 'import sys; print("ready",flush=True); sys.stdin.readline()', 'domain-child']
const parent = ['/usr/bin/python3', '-c', 'import subprocess,time; time.sleep(4); p=subprocess.Popen('+JSON.stringify(child)+'); p.wait()', 'domain-parent']
const writer = ['/usr/bin/python3', '-c', 'import os,sys; f=open("output","wb"); b=b"x"*1048576; [f.write(b) for _ in range(10240)]; f.flush(); os.fsync(f.fileno()); print("ready",flush=True); sys.stdin.readline()', 'domain-writer']
const second = ['/usr/bin/python3', '-c', 'import sys; print("ready",flush=True); sys.stdin.readline()', 'other-binary']
let mounted = false, ownsCgroup = false
class DomainFixture extends Fixture {
  async launch(name, binary, env, argv = []) {
    if (name === 'kernel-A') {
      if (disk) {
        execFileSync('truncate', ['-s', '32G', image])
        execFileSync('mkfs.ext4', ['-q', '-F', '-m', '0', image])
        execFileSync('mount', ['-o', 'loop', image, mountpoint]); mounted = true
        const space = await statfs(mountpoint); result.initial_free_bytes = space.bavail * space.bsize
      }
      const state = path.join(env.CHARIOX_HOME, 'state')
      await mkdir(state, {recursive: true})
      const peaks = disk ? [[writer, 64*mib, 12*gib], [second, 64*mib, 8*gib]] : [[parent, 4*gib, 0], [child, 4*gib, 0], [second, 4*gib, 0]]
      await writeFile(path.join(state, 'admission-peaks.json'), JSON.stringify(Object.fromEntries(peaks.map(([argv, memory, writes]) => [key(argv, mountpoint), {memory, writes}]))))
    }
    if (name.startsWith('kernel-')) {
      const group = path.join(cgroup, name); await mkdir(group)
      return super.launch(name, '/bin/bash', env, ['-c', 'echo "$$" > "$1"; shift; exec "$@"', 'binary-domains', path.join(group, 'cgroup.procs'), binary, ...argv])
    }
    return super.launch(name, binary, env, argv)
  }
  async cleanup() {
    // Fixture removes its home after stopping every owned process.
    if (mounted) {
      for (const row of this.children.filter(row => row.name.startsWith('kernel-'))) {
        signal(row.child, 'SIGTERM'); await row.child.done
      }
      execFileSync('umount', [mountpoint]); mounted = false
    }
    await super.cleanup()
  }
}
const fixture = new DomainFixture({home, evidence, kernelBinary:value('--kernel-bin'), relayBinary:value('--relay-bin'), cliBinary:value('--cli-bin')})
const active = new Set(), result = {mp:['MP-08','MP-09','MP-10','MP-11'], status:'FAIL', mode:disk?'disk':'descendants', scope:'real native CLI and kernels, private encrypted local relay; focused accounting acceptance only'}
function signal(child, name) {
  assert.ok(Number.isSafeInteger(child.pid) && child.pid > 1, 'invalid owned PID')
  if (child.exitCode === null && child.signalCode === null) child.kill(name)
}
function run(argv) {
  const proc = spawn(fixture.cliBinary, ['exec','--socket',fixture.envs.A.CHARIOX_DAEMON_SOCKET,'--session',result.session,'--',...argv], {env:fixture.envs.A, stdio:['pipe','pipe','pipe']})
  assert.ok(Number.isSafeInteger(proc.pid) && proc.pid > 1)
  active.add(proc); proc.output=''; proc.errors=''
  ;(result.native_pids ??= []).push({pid:proc.pid, command:argv.at(-1)})
  proc.stdout.on('data', bytes=>{proc.output+=bytes}); proc.stderr.on('data', bytes=>{proc.errors+=bytes}); proc.stdin.on('error',()=>{})
  proc.done = new Promise((resolve,reject)=>{proc.once('error',reject);proc.once('close',(code,signal)=>{active.delete(proc);resolve({code,signal})})});proc.done.catch(()=>{})
  return proc
}
async function finish(proc) {proc.stdin.end('done\n');assert.equal((await proc.done).code,0,proc.errors)}
try {
  await mkdir(evidence,{recursive:true}); await mkdir(cgroup); ownsCgroup=true
  await writeFile(path.join(cgroup,'memory.max'),String(10*gib)); await writeFile(path.join(cgroup,'memory.swap.max'),'0'); await writeFile(path.join(cgroup,'cgroup.subtree_control'),'+memory')
  await fixture.start(); await fixture.grant()
  result.session=(await fixture.A.send({CreateSession:{workspace_id:mountpoint,worktree_id:mountpoint,alias:'Binary domain accounting',no_subscribe:true}})).SessionCreated.session.id
  const first=run(disk?writer:parent)
  await until(()=>first.output==='ready\n','first binary produced output and remains running',120_000)
  if (disk) {
    result.written_bytes = (await stat(path.join(mountpoint, 'output'))).size
    assert.equal(result.written_bytes, 10*gib)
    const space = await statfs(mountpoint); result.free_bytes_after_output = space.bavail * space.bsize
  }
  // Let both normal resource watches consume the real child's/write counters.
  await new Promise(resolve=>setTimeout(resolve,1500))
  const other=run(second)
  await until(async()=>{
    result.states=(await fixture.A.send({RoomTerminal:{session_id:result.session,operation:{operation:'exec_list'}}})).RoomTerminal.result.execs.map(row=>({command:row.command.at(-1),state:row.state}))
    return other.output==='ready\n'
  },'second binary starts while first remains running',15_000)
  assert.equal(first.exitCode,null)
  await finish(other); await finish(first)
  result.status='PASS'; result.concurrent=true
} catch (error) {result.error=error.message;throw error}
finally {
  for (const proc of active) signal(proc,'SIGTERM')
  await Promise.allSettled([...active].map(proc=>proc.done)); await fixture.cleanup()
  const remove=async directory=>{for(const entry of await readdir(directory,{withFileTypes:true}))if(entry.isDirectory())await remove(path.join(directory,entry.name));await rmdir(directory)}
  if(ownsCgroup)await remove(cgroup)
  if(disk)await rm(image,{force:true})
  result.cleanup={cgroup_removed:true,image_removed:disk,mounted}
  await writeFile(path.join(evidence,'reservation-domains-results.json'),JSON.stringify(result,null,2))
}
