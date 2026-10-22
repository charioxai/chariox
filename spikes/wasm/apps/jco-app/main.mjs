// Option 1: the Rust component transpiled by jco, run inside today's Node
// worker. WIT imports map onto the real SDK; JSPI suspends the guest while an
// async SDK call is in flight.
import { readFile } from 'node:fs/promises';
import { WASIShim } from '@bytecodealliance/preview2-shim/instantiation';
import { instantiate } from './rust/app.js';

const modules = new Map();
function getCoreModule(path) {
  if (!modules.has(path)) modules.set(path, readFile(new URL(`./rust/${path}`, import.meta.url)).then(bytes => WebAssembly.compile(bytes)));
  return modules.get(path);
}

const appError = error => ({ payload: { code: String(error?.code ?? 'HANDLER_FAILED'), message: String(error?.message ?? ''), retryable: error?.retryable } });

function imports(chariox, names) {
  const call = async (operation) => { try { return await operation(); } catch (error) { throw appError(error); } };
  return {
    ...new WASIShim().getImportObject({ asVersion: '0.2.12' }),
    'chariox:app/types@0.1.0': {},
    'chariox:app/registry@0.1.0': { tool: name => names.push(name), event() {}, lifecycle() {} },
    'chariox:app/state@0.1.0': {
      get: key => call(async () => {
        const record = await chariox.state.get(key);
        return record ? { value: JSON.stringify(record.value), version: BigInt(record.version) } : undefined;
      }),
      transact: tx => call(async () => {
        const { revision } = await chariox.state.transaction({
          schemaVersion: tx.schemaVersion,
          checks: tx.checks.map(c => ({ key: c.key, version: c.version === undefined ? null : Number(c.version) })),
          writes: tx.writes.map(w => w.tag === 'put' ? { key: w.val[0], value: JSON.parse(w.val[1]) } : { key: w.val, delete: true }),
        });
        return { revision: BigInt(revision) };
      }),
    },
    'chariox:app/http@0.1.0': {
      send: req => call(async () => {
        const request = { url: req.url, method: req.method, headers: req.headers };
        if (req.body !== undefined) request.body = req.body;
        const response = await chariox.http.request(request);
        return { status: response.status, headers: response.headers, url: response.url, body: Buffer.from(response.bodyBase64, 'base64') };
      }),
    },
    'chariox:app/log@0.1.0': { write: async (level, message, fields) => { await chariox.log.write(level, message, JSON.parse(fields)).catch(() => {}); } },
  };
}

function context(ctx) {
  const optional = key => typeof ctx[key] === 'string' ? ctx[key] : undefined;
  return {
    deadlineMs: BigInt(ctx.deadlineMs), installationId: optional('installation_id'), roomId: optional('room_id'),
    operationId: optional('operation_id'), agentId: optional('agent_id'), taskId: optional('task_id'),
    turnId: optional('turn_id'), actor: ctx.actor ? { kind: ctx.actor.kind, id: ctx.actor.id } : undefined,
  };
}

export default async function register(chariox) {
  const names = [];
  const app = await instantiate(getCoreModule, imports(chariox, names));
  await app.handlers.register();
  for (const name of names) {
    chariox.tools.register(name, async (input, ctx) => {
      try {
        return JSON.parse(await app.handlers.callTool(name, JSON.stringify(input), context(ctx)));
      } catch (error) {
        const payload = error?.payload;
        if (payload?.code) throw new chariox.AppError(payload.code, payload.message);
        throw error;
      }
    });
  }
  chariox.tools.register('__warm', async ({ rounds }) => {
    const samplesMs = [];
    for (let i = 0; i < rounds; i++) {
      const at = performance.now();
      await instantiate(getCoreModule, imports(chariox, []));
      samplesMs.push(performance.now() - at);
    }
    return { samplesMs };
  });
}
