import { readFile, readdir, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

// strace starts before exec and follows every fork/clone, including short-lived
// children and threads. CDP/proc snapshots label live processes; clone ancestry
// labels threads that ended between snapshots. Unknown senders fail the gate.
export async function processNetwork(directory) {
  const snapshots = JSON.parse(await readFile(join(directory, 'pid-map.json'), 'utf8'));
  const identities = new Map(), ancestry = new Map(), files = [];
  for (const snapshot of snapshots) for (const process of snapshot.processes) {
    for (const tid of process.tasks ?? [process.pid]) identities.set(tid, { pid: process.pid, type: process.type });
  }
  for (const file of await readdir(directory)) {
    if (!/^syscall\.\d+$/.test(file)) continue;
    const tid = Number(file.split('.')[1]);
    const content = await readFile(join(directory, file), 'utf8');
    files.push({ file, tid, lines: content.split('\n') });
    for (const line of content.split('\n')) {
      if (/\b(?:clone|clone3|fork|vfork)(?:\(| resumed>)/.test(line)) {
        const child = /\)\s+= (\d+)(?:\s|$)/.exec(line);
        if (child) ancestry.set(Number(child[1]), { tid, thread: line.includes('CLONE_THREAD') });
      }
    }
  }
  function identity(tid, seen = new Set()) {
    if (identities.has(tid)) return identities.get(tid);
    if (seen.has(tid)) return { pid: tid, type: 'UNKNOWN' };
    seen.add(tid);
    const parent = ancestry.get(tid);
    if (!parent) return { pid: tid, type: 'UNKNOWN' };
    const owner = identity(parent.tid, seen);
    return parent.thread ? owner : { pid: tid, type: `child-of-${owner.type}` };
  }
  const incomplete = files.filter(file => !/\+\+\+ (?:exited with|killed by).+\+\+\+$/.test(file.lines.filter(Boolean).at(-1) ?? '')).map(file => file.file);
  const outgoing = [], dnsQueries = [], unknown = [], destinations = new Map();
  const ordered = files.flatMap(({ file, tid, lines }) => lines.map(line => ({ file, tid, line })))
    .sort((a, b) => Number(a.line.split(' ')[0]) - Number(b.line.split(' ')[0]));
  for (const { file, tid, line } of ordered) {
    const close = /\bclose\((\d+)</.exec(line);
    if (close) { destinations.delete(identity(tid).pid + ':' + close[1]); continue; }
    if (!/\b(?:connect|sendto|sendmsg|sendmmsg)\(/.test(line)) continue;
    const matches = [...line.matchAll(/sin_port=htons\((\d+)\), sin_addr=inet_addr\("([^"]+)"\)/g),
      ...line.matchAll(/sin6_port=htons\((\d+)\)[^}]*?inet_pton\(AF_INET6, "([^"]+)"/g)];
    let match = matches[0];
    if (!match) {
      const fd = /<[^>]*->(?:\[([^\]]+)\]|([^:>]+)):(\d+)\]>/.exec(line);
      if (fd) match = [fd[0], fd[3], fd[1] ?? fd[2]];
    }
    const fd = /\((\d+)</.exec(line)?.[1], owner = identity(tid);
    const socketKey = owner.pid + ':' + fd;
    if (!match && fd && destinations.has(socketKey) && /<(?:TCP|UDP)(?:v6)?:/.test(line)) {
      const prior = destinations.get(socketKey); match = ['', prior.port, prior.address];
    }
    if (!match) {
      if (/sa_family=AF_INET6?\b/.test(line) || /<(?:TCP|UDP)(?:v6)?:/.test(line) && !/\bconnect\(/.test(line)) unknown.push({ tid, ...identity(tid), file, line });
      continue;
    }
    for (const destination of matches.length ? matches : [match]) {
      const port = Number(destination[1]), address = destination[2];
      destinations.set(socketKey, { port, address });
      const row = { time: Number(line.split(' ')[0]), tid, ...identity(tid), address, port, file, line };
      outgoing.push(row);
      if ([53, 853].includes(port) && /\b(?:sendto|sendmsg|sendmmsg)\(/.test(line)) dnsQueries.push(row);
    }
  }
  const loopback = address => /^127\./.test(address) || address === '::1' || /^::ffff:127\./.test(address);
  const nonLoopback = outgoing.filter(row => !loopback(row.address));
  const unidentified = outgoing.filter(row => row.type === 'UNKNOWN' || row.type.startsWith('child-of-'));
  const result = { syscallFiles: files.length, processTree: [...new Set(files.map(file => file.tid))].map(tid => ({ tid, ...identity(tid), parent: ancestry.get(tid) })),
    outgoing, nonLoopback, dnsQueries, unknown, unidentified, incomplete,
    passed: files.length > 0 && incomplete.length === 0 && outgoing.length > 0 && nonLoopback.length === 0 && dnsQueries.length === 0 && unknown.length === 0 && unidentified.length === 0 };
  await writeFile(join(directory, 'network.json'), JSON.stringify(result, null, 2));
  return { directory, syscallFiles: files.length, nonLoopback: nonLoopback.length, dnsQueries: dnsQueries.length, unknown: unknown.length,
    unidentified: unidentified.length, incomplete: incomplete.length, passed: result.passed };
}
