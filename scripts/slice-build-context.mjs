// MP-07 / MP-08 / MP-10 / MP-11: release preflight checks the distributed
// context, never the builder checkout. This resolves local inputs; it does not
// claim that the complete image or its runtime has passed live acceptance.
import { glob, lstat, readFile } from 'node:fs/promises';
import { join } from 'node:path';

export const SLICE_DOCKERFILE = 'apps/kernel/slice-linux-docker/docker/Dockerfile';

const fail = message => { throw new Error(`slice build context: ${message}`); };

function localPath(source) {
  const path = source.replace(/^\/+/, '').replace(/^(?:\.\/)+/, '').replace(/\/+$/, '');
  // Do not silently approximate environment expansion, heredocs, remote ADD,
  // or advanced glob syntax. Extend the preflight before introducing these.
  if (!path || path.split('/').some(part => !part || part === '..' || part === '.')
    || /[$<\\:{}!()]/.test(path) || path.includes('**'))
    fail(`unsupported local input ${source}`);
  return path;
}

function words(text) {
  const tokens = [];
  let token = '', quote = null, escaped = false;
  for (const character of text.trim()) {
    if (escaped) { token += character; escaped = false; }
    else if (character === '\\' && quote !== "'") escaped = true;
    else if (quote) {
      if (character === quote) quote = null;
      else token += character;
    } else if (character === "'" || character === '"') quote = character;
    else if (/\s/.test(character)) {
      if (token) tokens.push(token);
      token = '';
    } else token += character;
  }
  if (quote || escaped) fail('unterminated quote or escape in COPY/ADD');
  if (token) tokens.push(token);
  return tokens;
}

/** Local COPY/ADD and RUN bind inputs, including every build stage. */
export function sliceDockerfileInputs(dockerfile) {
  if (/^\s*#\s*escape\s*=\s*[^\\\s]/mi.test(dockerfile)) fail('unsupported Dockerfile escape directive');
  const lines = dockerfile.split(/\r?\n/).filter(line => !/^\s*#/.test(line)).join('\n')
    .replace(/\\\r?\n\s*/g, ' ').split('\n');
  const inputs = [];
  for (const line of lines) {
    const copy = /^\s*(COPY|ADD)\s+(.+)$/i.exec(line);
    if (copy) {
      let text = copy[2].trim(), from = false;
      while (text.startsWith('--')) {
        const option = /^--([a-z-]+)(?:=([^\s]+))?\s+/i.exec(text);
        if (!option) fail('unsupported COPY/ADD option');
        if (!option[2] && !['link', 'parents'].includes(option[1])) fail('COPY/ADD option needs an explicit value');
        if (option[1] === 'from') from = true;
        text = text.slice(option[0].length).trimStart();
      }
      const tokens = text.startsWith('[') ? JSON.parse(text) : words(text);
      if (!Array.isArray(tokens) || tokens.length < 2 || tokens.some(token => typeof token !== 'string' || !token))
        fail('COPY/ADD needs sources and a destination');
      if (!from) inputs.push(...tokens.slice(0, -1).map(localPath));
    } else if (/^\s*RUN\s/i.test(line)) {
      for (const match of line.matchAll(/--mount=([^\s]+)/g)) {
        const options = new Map(match[1].split(',').map(part => {
          const index = part.indexOf('=');
          return index < 0 ? [part, ''] : [part.slice(0, index), part.slice(index + 1)];
        }));
        if ((options.get('type') ?? 'bind') !== 'bind' || options.has('from')) continue;
        inputs.push(localPath(options.get('source') ?? options.get('src') ?? '.'));
      }
    }
  }
  if (!inputs.length) fail('Dockerfile has no local inputs');
  return [...new Set(inputs)];
}

/** A Git archive pathspec that carries all matches of a Docker source glob. */
export function sliceInputRoot(input) {
  const index = input.search(/[*?[]/);
  if (index < 0) return input;
  const root = input.slice(0, index).split('/').slice(0, -1).join('/');
  if (!root) fail(`glob must have an explicit directory: ${input}`);
  return root;
}

export async function validateSliceBuildContext(directory) {
  // The exporter does not ship ignore files. Refuse externally supplied ignore
  // rules rather than claiming existence proves Docker will receive the file.
  for (const path of ['.dockerignore', `${SLICE_DOCKERFILE}.dockerignore`]) {
    if (await lstat(join(directory, path)).then(() => true, () => false)) fail(`unsupported ignore file ${path}`);
  }
  const dockerfile = await readFile(join(directory, SLICE_DOCKERFILE), 'utf8');
  const inputs = sliceDockerfileInputs(dockerfile);
  for (const source of inputs) {
    const matches = /[*?[]/.test(source) ? glob(source, { cwd: directory }) : [source];
    let found = false;
    for await (const path of matches) {
      const metadata = await lstat(join(directory, path)).catch(error => {
        if (error.code === 'ENOENT') return null;
        throw error;
      });
      if (!metadata) continue;
      if (!metadata.isFile() && !metadata.isDirectory()) fail(`input is a link or special file: ${source}`);
      found = true;
    }
    if (!found) fail(`missing Dockerfile input ${source}`);
  }
  return { dockerfile: SLICE_DOCKERFILE, inputs: inputs.length };
}
