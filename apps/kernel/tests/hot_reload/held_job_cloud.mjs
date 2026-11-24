import { createHmac, randomUUID } from 'node:crypto';
import { startOwnerCloud } from '../../../../scripts/drills/local-owner-cloud.test-support.mjs';

const { HELD_JOB_RELAY_PORT, HELD_JOB_TEST_RELAY_SECRET } = process.env;
const realm = 'held-job-reload';
const machine = 'held-job-worker';
const allocation = 'held-job-machine';
const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url');
const token = (subject, kind, actions, thumbprint, targets) => {
  const now = Math.floor(Date.now() / 1000);
  const scopes = kind === 'client' && actions.includes('daemon.register')
    ? ['client.connect', 'client.metadata.read', 'packet.route'] : actions;
  const claims = {
    iss: realm, sub: subject, subject_kind: kind, realm_id: realm,
    allowed_actions: scopes, allowed_targets: targets ?? null,
    iat: now - 1, exp: now + 600, jti: randomUUID(), user_id: 'local',
  };
  if (thumbprint) claims.public_key_thumbprint = thumbprint;
  const body = encode({ alg: 'HS256', typ: 'JWT' }) + '.' + encode(claims);
  return body + '.' + createHmac('sha256', HELD_JOB_TEST_RELAY_SECRET).update(body).digest('base64url');
};

let server;
let worker;
const cloud = await startOwnerCloud({
  token, realm, relayPort: HELD_JOB_RELAY_PORT, note: () => {},
  onServer: instance => { server = instance; },
  onPresence: ({ kernelId, keyThumbprint, machineId }) => {
    // The Rust fixture connects the worker before it starts the home kernel. The machine id is the
    // one the worker registered: the home kernel checks the target against relay discovery, and a
    // machine whose enrollment and discovery disagree is never ready.
    worker ??= { kernel_id: kernelId, machine_id: machineId, key_thumbprint: keyThumbprint, relay_realm_id: realm };
  },
});
const api = server.listeners('request')[0];
server.removeListener('request', api);
server.on('request', async (req, res) => {
  const send = (status, payload) => res.writeHead(status, { 'content-type': 'application/json' }).end(JSON.stringify(payload));
  if (req.url === '/kernels/gateway-client' && req.method === 'POST') {
    req.resume();
    return send(200, {});
  }
  if (req.url !== '/control-plane') return api(req, res);
  try {
    const chunks = [];
    for await (const chunk of req) chunks.push(chunk);
    const { request } = JSON.parse(Buffer.concat(chunks).toString('utf8'));
    if (request.action === 'capabilities') return send(200, { kind: 'capabilities', machine_allocation: null });
    if (request.action === 'get' && request.allocation_ref === allocation && worker) {
      return send(200, {
        kind: 'machine', revision: 1, state: 'ready', available_actions: [], allocation_ref: allocation,
        access_state: 'pending', machine_label: 'held job test worker', kernel_target: worker,
      });
    }
    return send(404, { error: { code: 'not_found' } });
  } catch {
    return send(400, {});
  }
});
server.ref();
const profile = JSON.parse(cloud.CHARIOX_CLOUD_RELAY_CONFIG_JSON);
process.stdout.write(JSON.stringify({ home: profile, worker: { ...profile, machine_id: machine } }) + '\n');
process.stdin.resume();
process.stdin.on('end', () => { server.closeAllConnections(); server.close(); });
