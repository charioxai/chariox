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
  const outgoing = [], dnsQueries = [], unknown = [], destinations = new Map(), sockets = new Map();
  const socketOperations = [];
  const ordered = files.flatMap(({ file, tid, lines }) => lines.map(line => ({ file, tid, line })))
    .sort((a, b) => Number(a.line.split(' ')[0]) - Number(b.line.split(' ')[0]));
  for (const { file, tid, line } of ordered) {
    const owner = identity(tid);
    const created = /\bsocket\((AF_INET6?), (SOCK_DGRAM|SOCK_STREAM)\b[^)]*\)\s+= (\d+)/.exec(line);
    if (created) sockets.set(owner.pid + ':' + created[3], { family: created[1], socketType: created[2],
      socketId: `${owner.pid}:${created[3]}:${line.split(' ')[0]}` });
    const duplicate = /\b(?:dup[23]?\((\d+)(?:<[^>]*>)?[^)]*|fcntl\((\d+)(?:<[^>]*>)?, F_DUPFD[^)]*)\)\s+= (\d+)/.exec(line);
    if (duplicate) {
      const source = owner.pid + ':' + (duplicate[1] ?? duplicate[2]), target = owner.pid + ':' + duplicate[3];
      if (sockets.has(source)) sockets.set(target, sockets.get(source));
      if (destinations.has(source)) destinations.set(target, destinations.get(source));
    }
    const close = /\bclose\((\d+)</.exec(line);
    if (close) { destinations.delete(owner.pid + ':' + close[1]); sockets.delete(owner.pid + ':' + close[1]); continue; }
    const operation = /\b(connect|sendto|sendmsg|sendmmsg|write|writev|sendfile|splice)\(/.exec(line)?.[1];
    if (!operation) continue;
    const fd = (operation === 'splice' ? /\bsplice\(\d+(?:<[^>]*>)?, [^,]+, (\d+)/ : /\((\d+)/).exec(line)?.[1];
    const socketKey = owner.pid + ':' + fd, socket = sockets.get(socketKey);
    if (socket) socketOperations.push({ socketId: socket.socketId, operation, tid, ...owner, file, line });
    if (['write', 'writev', 'sendfile', 'splice'].includes(operation) && !socket && !/<(?:TCP|UDP)(?:v6)?:/.test(line)) continue;
    const matches = [...line.matchAll(/sin_port=htons\((\d+)\), sin_addr=inet_addr\("([^"]+)"\)/g),
      ...line.matchAll(/sin6_port=htons\((\d+)\)[^}]*?inet_pton\(AF_INET6, "([^"]+)"/g)];
    let match = matches[0];
    if (!match) {
      const fd = /<[^>]*->(?:\[([^\]]+)\]|([^:>]+)):(\d+)\]>/.exec(line);
      if (fd) match = [fd[0], fd[3], fd[1] ?? fd[2]];
    }
    if (!match && fd && destinations.has(socketKey)) {
      const prior = destinations.get(socketKey); match = ['', prior.port, prior.address];
    }
    if (!match) {
      if (socket || /sa_family=AF_INET6?\b/.test(line) || /<(?:TCP|UDP)(?:v6)?:/.test(line) && operation !== 'connect') unknown.push({ tid, ...owner, file, line });
      continue;
    }
    for (const destination of matches.length ? matches : [match]) {
      const port = Number(destination[1]), address = destination[2];
      destinations.set(socketKey, { port, address });
      const row = { time: Number(line.split(' ')[0]), tid, ...owner, address, port, file, line, operation, ...socket,
        result: /\)\s+= (.*)$/.exec(line)?.[1] };
      outgoing.push(row);
      if ([53, 853].includes(port) && operation !== 'connect') dnsQueries.push(row);
    }
  }
  const loopback = address => /^127\./.test(address) || address === '::1' || /^::ffff:127\./.test(address);
  const nonLoopback = outgoing.filter(row => !loopback(row.address));
  const unidentified = outgoing.filter(row => row.type === 'UNKNOWN' || row.type.startsWith('child-of-'));
  const result = { syscallFiles: files.length, processTree: [...new Set(files.map(file => file.tid))].map(tid => ({ tid, ...identity(tid), parent: ancestry.get(tid) })),
    outgoing, nonLoopback, dnsQueries, unknown, unidentified, incomplete, socketOperations,
    passed: files.length > 0 && incomplete.length === 0 && outgoing.length > 0 && nonLoopback.length === 0 && dnsQueries.length === 0 && unknown.length === 0 && unidentified.length === 0 };
  await writeFile(join(directory, 'network.json'), JSON.stringify(result, null, 2));
  return { directory, syscallFiles: files.length, nonLoopback: nonLoopback.length, dnsQueries: dnsQueries.length, unknown: unknown.length,
    unidentified: unidentified.length, incomplete: incomplete.length, passed: result.passed,
    nonLoopbackOperations: nonLoopback, socketOperations };
}
