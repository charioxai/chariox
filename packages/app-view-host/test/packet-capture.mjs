// MP-08 / MP-09 / MP-10 / MP-11 — wire corroboration for the sole socket exception.
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { routeProbeException } from './network-assessment.mjs';

const run = promisify(execFile);
export const routeProbeFilter = `ip6 and dst host ${routeProbeException.address}`;

export async function assessPackets(evidence, phases, strictFilter) {
  const capture = JSON.parse(await readFile(join(evidence, 'capture-process.json'), 'utf8'));
  const log = await readFile(join(evidence, 'tcpdump.log'), 'utf8');
  const dropped = /([0-9]+) packets dropped by kernel/.exec(log);
  async function packets(filter) {
    const { stdout } = await run('tcpdump', ['-nn', '-tt', '-r', join(evidence, 'receiver.pcap'), filter], { maxBuffer: 16 * 1024 * 1024 });
    return stdout.trim().split('\n').filter(Boolean).map(line => {
      const time = Number(line.split(' ')[0]) * 1000;
      if (!Number.isFinite(time)) throw new Error('Packet capture has an unreadable timestamp');
      return { time, line, phase: phases.findLast(phase => phase.time <= time)?.name ?? 'initialization' };
    });
  }
  const external = await packets(routeProbeFilter), receiver = await packets(strictFilter);
  const result = { complete: capture.state === 'exited' && capture.exitCode === 0 && dropped !== null,
    dropped: dropped ? Number(dropped[1]) : null,
    // The syscall assessment rejects every other non-loopback destination. Only
    // the allowed address can survive it, so its wire capture must be empty.
    nonLoopbackPackets: external.length, strictReceiverPackets: receiver.filter(row => !row.phase.endsWith('-control')).length,
    nonLoopbackFilter: routeProbeFilter, strictFilter, external, receiver, phases };
  await writeFile(join(evidence, 'packet-assessment.json'), JSON.stringify(result, null, 2));
  return result;
}
