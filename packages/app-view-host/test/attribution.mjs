import { readFile, writeFile } from 'node:fs/promises';
import { join } from 'node:path';

// This report never grants an exception. The unchanged zero-socket gate uses
// network.passed; a narrower candidate is evidence for an explicit owner choice.
export async function attribution(evidence, results) {
  const rows = [];
  const runs = results.attacks.flatMap(item => item.runs.map(run => ({ name: item.name, dpr: item.dpr, ...run })));
  for (const run of [...runs, ...results.references, ...(results.isolationNetwork ? [{ name: 'isolation-and-binding', dpr: 1, processNetwork: results.isolationNetwork }] : []), ...(results.launch?.rows ?? []).map(row => ({ ...row, name: 'launch-' + row.kind, dpr: 1, processNetwork: row.network }))]) {
    if (!run.processNetwork) continue;
    const directory = run.processNetwork.directory;
    const network = JSON.parse(await readFile(join(directory, 'network.json'), 'utf8'));
    const snapshots = JSON.parse(await readFile(join(directory, 'pid-map.json'), 'utf8'));
    const names = new Set(), appDns = [];
    if (run.control !== undefined) {
      const phase = run.phase ?? `${run.name}-dpr${run.dpr}-${run.control ? 'control' : 'strict'}`;
      const netlog = JSON.parse(await readFile(join(evidence, 'netlog', phase + '.json'), 'utf8'));
      const type = netlog.constants.logEventTypes.DNS_TRANSACTION_QUERY;
      for (const event of netlog.events) if (event.type === type && event.params?.qname) names.add(event.params.qname);
      for (const event of network.dnsQueries) if (event.line.includes('appview-dns-')) appDns.push(event);
    }
    const routeProbe = row => row.type === 'network.mojom.NetworkService' && row.address === '2001:4860:4860::8888'
      && row.port === 443 && /\bconnect\(\d+<UDPv6:/.test(row.line);
    rows.push({ name: run.name, dpr: run.dpr, control: run.control ?? false, directory,
      processFrames: snapshots.at(-1).frames,
      outgoingProcesses: [...new Map(network.outgoing.map(row => [row.pid, { pid: row.pid, type: row.type }])).values()],
      dnsNames: [...names], frameInitiatedDns: appDns, nonLoopback: network.nonLoopback,
      dnsQueries: network.dnsQueries.length, unknown: network.unknown.length, unidentified: network.unidentified.length, incomplete: network.incomplete.length,
      zeroNetworkGatePassed: network.passed,
      diagnosticRouteProbeOnly: network.nonLoopback.length > 0 && network.nonLoopback.every(routeProbe)
        && network.dnsQueries.length === 0 && network.unknown.length === 0 && network.unidentified.length === 0 && network.incomplete.length === 0 });
  }
  const report = { scope: results.scope, diagnosticOnly: true, accepted: false,
    proposedException: 'Network Service UDP connect only to [2001:4860:4860::8888]:443, Chromium IPv6 route probe; no send call permitted. This is NOT accepted by the zero-network gate.', rows };
  await writeFile(join(evidence, 'attribution.json'), JSON.stringify(report, null, 2));
  return { strictRuns: rows.filter(row => !row.control).length,
    strictZeroNetworkPassed: rows.filter(row => !row.control && row.zeroNetworkGatePassed).length,
    strictRouteProbeOnly: rows.filter(row => !row.control && row.diagnosticRouteProbeOnly).length };
}
