'use strict';

const { realpathSync, statSync } = require('node:fs');
const path = require('node:path');

function object(value, keys) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
}

function root(value) {
  if (typeof value !== 'string' || !path.isAbsolute(value) || value === '/'
    || Buffer.byteLength(value) > 1024 || /[\x00-\x1f\x7f]/u.test(value)
    || realpathSync(value) !== value || !statSync(value).isDirectory()) throw new Error('configuration');
  return value;
}

// These inputs originate in the native launch record and the trusted kernel's
// serialized bootstrap. This validates the ABI, never grants permissions.
exports.configuration = function configuration(input, environment, runtime) {
  if (!object(input, ['version', 'entry', 'declarations', 'startupTimeoutMs']) || input.version !== 1
    || !Number.isSafeInteger(input.startupTimeoutMs) || input.startupTimeoutMs < 1 || input.startupTimeoutMs > 15000
    || !object(input.declarations, ['tools', 'events'])) throw new Error('configuration');
  for (const names of Object.values(input.declarations)) {
    if (!Array.isArray(names) || names.length > 1024 || new Set(names).size !== names.length
      || names.some(name => typeof name !== 'string' || !name || Buffer.byteLength(name) > 128
        || /[\p{Cc}\p{White_Space}\uFEFF]/u.test(name))) throw new Error('configuration');
  }
  const generation = environment.CHARIOX_APP_GENERATION;
  if (typeof generation !== 'string' || !/^[1-9][0-9]{0,18}$/u.test(generation)
    || BigInt(generation) > 9223372036854775807n
    || !/^[a-zA-Z0-9_.-]{1,128}$/u.test(environment.CHARIOX_APP_INSTALLATION ?? '')
    || !/^[a-f0-9]{64}$/u.test(environment.CHARIOX_APP_RELEASE_DIGEST ?? '')) throw new Error('configuration');
  const paths = {
    package: root(environment.CHARIOX_APP_PACKAGE),
    data: root(environment.CHARIOX_APP_DATA),
    temporary: root(environment.CHARIOX_APP_TMP),
  };
  const roots = [...Object.values(paths), root(runtime)];
  for (let index = 0; index < roots.length; ++index) {
    if (roots.slice(0, index).some(other => roots[index] === other
      || roots[index].startsWith(`${other}/`) || other.startsWith(`${roots[index]}/`))) throw new Error('configuration');
  }
  if (typeof input.entry !== 'string' || !input.entry.startsWith('runtime/')
    || Buffer.byteLength(input.entry) > 1024 || !/\.(?:js|mjs|cjs)$/u.test(input.entry)
    || /[\\\x00-\x1f\x7f]/u.test(input.entry)
    || input.entry.split('/').some(part => part === '' || part === '.' || part === '..')) throw new Error('configuration');
  const entry = path.join(paths.package, input.entry);
  // No symlink or alternate root can redirect even the initial App import.
  // The supervisor holds the entire verified package immutable thereafter.
  if (realpathSync(entry) !== entry || !statSync(entry).isFile()) throw new Error('configuration');
  return Object.freeze({ generation, entry, paths: Object.freeze(paths),
    declarations: Object.freeze({ tools: Object.freeze([...input.declarations.tools]), events: Object.freeze([...input.declarations.events]) }),
    startupTimeoutMs: input.startupTimeoutMs });
};
