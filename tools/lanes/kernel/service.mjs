import { readFileSync } from 'node:fs';
// MP-08 / MP-10 / MP-11: boot startup uses the same lanes start/stop path as the coordinator.
export function sliceInvocation({ sliceExists, cgroup, node, script, args }) {
  if (!sliceExists || /(?:^|\/)lanes\.slice(?:\/|$)/m.test(cgroup)) return undefined;
  return ['systemd-run', '--quiet', '--slice=lanes.slice', '--scope', '--', node, script, ...args];
}
const path = value => {
  if (typeof value !== 'string' || !value.startsWith('/') || /[\n\r\0%]/.test(value)) throw new Error('service path must be absolute without newlines or systemd specifiers');
  return value.replaceAll('\\', '\\\\').replaceAll('"', '\\"');
};
export function serviceUnit({ node, script, state }) {
  return readFileSync(new URL('./chariox-lanes.service', import.meta.url), 'utf8')
    .replaceAll('@STATE@', path(state)).replaceAll('@NODE@', path(node)).replaceAll('@SCRIPT@', path(script));
}
