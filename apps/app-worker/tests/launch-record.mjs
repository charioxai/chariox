// Shared test encoder for the production internal ABI in src/launcher.md.
export function launchString(value) {
  const bytes = Buffer.from(value);
  const size = Buffer.alloc(4);
  size.writeUInt32BE(bytes.length);
  return Buffer.concat([size, bytes]);
}

export function launchRecord(roots, overrides = {}) {
  const options = { generation: '1', installation: 'probe-install', digest: 'a'.repeat(64),
    bootstrap: 'probe-v1', ...overrides };
  const limits = Buffer.alloc(24);
  limits.writeUInt32BE(64, 0);
  limits.writeUInt32BE(10, 4);
  limits.writeUInt32BE(64, 8);
  limits.writeUInt32BE(1, 12);
  limits.writeBigUInt64BE(1024n * 1024n, 16);
  const body = Buffer.concat([limits, ...[
    options.generation, options.installation, options.digest,
    roots.package, roots.data, roots.tmp, roots.runtime, options.bootstrap,
  ].map(launchString)]);
  const header = Buffer.alloc(12);
  header.write('CXAWL001');
  header.writeUInt32BE(body.length, 8);
  return Buffer.concat([header, body]);
}

export const expectedReadiness = Buffer.concat([
  Buffer.from('CXAWR001'), launchString('1'), launchString('probe-install'), launchString('a'.repeat(64)),
]);
