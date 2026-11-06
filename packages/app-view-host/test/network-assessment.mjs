// MP-08 / MP-09 / MP-10 / MP-11 — Miguel's 2026-11-05 exception, exactly.
export const routeProbeException = Object.freeze({
  name: 'chromium-network-service-ipv6-route-probe',
  processType: 'network.mojom.NetworkService', family: 'AF_INET6', socketType: 'SOCK_DGRAM',
  address: '2001:4860:4860::8888', port: 443,
});

export function packetsPassed(packets) {
  return packets?.complete === true && packets.dropped === 0 && packets.nonLoopbackPackets === 0 && packets.strictReceiverPackets === 0;
}

export function assessNetwork(network, packets) {
  if (network.passed) return { passed: true, exemptions: [] };
  const operations = network.nonLoopbackOperations;
  const cleanCapture = network.syscallFiles > 0 && network.incomplete === 0 && network.unidentified === 0
    && network.unknown === 0 && network.dnsQueries === 0 && packetsPassed(packets);
  const exactProbe = row => row.type === routeProbeException.processType && row.family === routeProbeException.family
    && row.socketType === routeProbeException.socketType && row.address === routeProbeException.address
    && row.port === routeProbeException.port && row.operation === 'connect' && row.socketId && row.result
    && network.socketOperations?.some(call => call.socketId === row.socketId && call.operation === 'connect')
    && network.socketOperations.every(call => call.socketId !== row.socketId || call.operation === 'connect');
  if (!cleanCapture || !operations?.length || !operations.every(exactProbe)) return { passed: false, exemptions: [] };
  return { passed: true, exemptions: operations.map(row => ({ name: routeProbeException.name, ...row })) };
}
