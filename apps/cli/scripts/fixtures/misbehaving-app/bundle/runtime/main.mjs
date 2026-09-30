// Runtime-limit drill (V-RUN-01/02/03/05/08): each `misbehave` occurrence makes
// the worker misbehave in one way, so the kernel's limits can be observed.
//   ok     log and acknowledge (control)
//   spin   a synchronous infinite loop (CPU)
//   grow   keep allocating memory
//   flood  write log entries far past the rate limit
//   flood-secrets  the same, each entry carrying secret-shaped strings that
//          the kernel must redact before storing them (V-RUN-08)
//   hang   never acknowledge the event
//   crash  throw outside any handler

// Fake secret-shaped samples, assembled at run time so the package holds no
// literal token for a secret scanner to flag.
const pad = (length) => 'a1B2c3D4e5'.repeat(10).slice(0, length);
const SECRETS = [
  `openai sk-proj-${pad(40)}`,
  `anthropic sk-ant-api03-${pad(40)}`,
  `slack xoxb-${'1'.repeat(12)}-${pad(24)}`,
  `github ghp_${pad(36)} and github_pat_${pad(22)}_${pad(59)}`,
  `aws AKIA${'IOSFODNN7EXAMPLE'} aws_secret_access_key=${pad(40)}`,
  `google AIza${pad(35)}`,
  `jwt eyJ${pad(20)}.eyJ${pad(30)}.${pad(43)}`,
  `header Authorization: Bearer ${pad(32)}`,
  `pem -----BEGIN ${'RSA PRIVATE'} KEY-----\n${'A'.repeat(64)}\n-----END ${'RSA PRIVATE'} KEY-----`,
  `config password=${pad(12)} token=${pad(16)} secret: ${pad(10)}`,
];

async function flood(chariox, entry) {
  const writes = [];
  for (let i = 0; i < 5000; i += 1) {
    const [message, fields] = entry(i);
    // `log.write` resolves null; a refused write rejects with a code.
    writes.push(chariox.log.write('info', message, fields).then(() => null, (error) => error?.code ?? 'failed'));
  }
  const results = await Promise.all(writes);
  const refused = results.filter((value) => value !== null).length;
  await chariox.log.write('warn', 'flood finished', { attempted: 5000, refused });
}

export default function register(chariox) {
  chariox.events.register('misbehave', async ({ payload }) => {
    if (payload.mode === 'ok') {
      await chariox.log.write('info', 'ok');
    } else if (payload.mode === 'spin') {
      for (;;) {}
    } else if (payload.mode === 'grow') {
      const held = [];
      for (;;) {
        held.push(Buffer.alloc(16 * 1024 * 1024, 1));
        await new Promise((resolve) => setTimeout(resolve, 5));
      }
    } else if (payload.mode === 'flood') {
      await flood(chariox, (i) => [`flood ${i}`, { i }]);
    } else if (payload.mode === 'flood-secrets') {
      await flood(chariox, (i) => {
        const secret = SECRETS[i % SECRETS.length];
        return [`flood ${i} ${secret}`, { i, sample: secret, api_key: pad(20) }];
      });
    } else if (payload.mode === 'hang') {
      await new Promise(() => {});
    } else if (payload.mode === 'crash') {
      setTimeout(() => { throw new Error('drill crash'); }, 0);
    }
  });
}
