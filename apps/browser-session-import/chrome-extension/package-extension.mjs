import {copyFile,mkdir,readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

const repository = fileURLToPath(new URL('../../..',import.meta.url));
const extensionSource = fileURLToPath(new URL('.',import.meta.url));

export async function packageChromeExtension(output,typescript,{extensionPublicKey} = {}) {
  if (!path.isAbsolute(output) || !typescript?.transpileModule) throw new Error('usage: package-extension.mjs ABSOLUTE_OUTPUT_DIRECTORY');
  const extensionTarget = path.join(output,'apps/browser-session-import/chrome-extension');
  const importTarget = path.join(output,'apps/browser-session-import');
  const clientTarget = path.join(output,'packages/kernel-client/src');
  await mkdir(extensionTarget,{recursive:true});
  await mkdir(clientTarget,{recursive:true});
  for (const file of ['background.mjs','connector.html','connector.css','connector.mjs',
    'connector-core.mjs','delivery-adapter.mjs','permission-coordinator.mjs',
    'external-port-broker.mjs','web-bridge-protocol.mjs']) {
    await copyFile(path.join(extensionSource,file),path.join(extensionTarget,file));
  }
  const manifest=JSON.parse(await readFile(path.join(extensionSource,'manifest.json'),'utf8'));
  const extensionId=extensionPublicKey ? extensionIdForPublicKey(extensionPublicKey) : null;
  if (extensionId) {
    manifest.key=extensionPublicKey;
    await writeFile(path.join(output,'browser-import-extension-id.txt'),`${extensionId}\n`);
  }
  await writeFile(path.join(output,'manifest.json'),`${JSON.stringify(manifest,null,2)}\n`);
  for (const file of ['chrome-cookie-batch.mjs','chrome-cookie-reader.mjs','kernel-source-reader.mjs',
    'relay-request-metadata.mjs','chrome-import-consent-flow.mjs','relay-connector.mjs','relay-response.mjs']) {
    const source = await readFile(path.join(repository,'apps/browser-session-import',file),'utf8');
    await writeFile(path.join(importTarget,file),source.replaceAll(
      '../../packages/kernel-client/src/browser-import-requests.ts',
      '../../packages/kernel-client/src/browser-import-requests.js').replaceAll(
      '../../packages/kernel-client/src/browser-relay-crypto.ts',
      '../../packages/kernel-client/src/browser-relay-crypto.js'));
  }
  for (const file of ['browser-import-requests.ts','browser-relay-crypto.ts','kernel-transport-frames.ts']) {
    const source = await readFile(path.join(repository,'packages/kernel-client/src',file),'utf8');
    const compiled = typescript.transpileModule(source,{compilerOptions:{
      module:typescript.ModuleKind.ES2022,target:typescript.ScriptTarget.ES2022,
    }}).outputText;
    await writeFile(path.join(clientTarget,file.replace(/\.ts$/,'.js')),compiled);
  }
  return Object.freeze({extensionId});
}

export function extensionIdForPublicKey(value) {
  if (typeof value !== 'string' || value.length < 44 || value.length > 16384
      || !/^[A-Za-z0-9+/]+={0,2}$/.test(value)) throw new Error('invalid Chrome extension public key');
  const bytes=Buffer.from(value,'base64');
  if (bytes.length < 32 || bytes.toString('base64') !== value) throw new Error('invalid Chrome extension public key');
  return [...createHash('sha256').update(bytes).digest().subarray(0,16)]
    .flatMap(byte=>[byte>>4,byte&15]).map(nibble=>String.fromCharCode(97+nibble)).join('');
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const output = process.argv[2];
  const compiler = process.env.TYPESCRIPT_MODULE;
  if (!output || !compiler) throw new Error('set TYPESCRIPT_MODULE and pass an absolute output directory');
  await packageChromeExtension(output,(await import(compiler)).default,
    {extensionPublicKey:process.env.CHARIOX_BROWSER_IMPORT_EXTENSION_PUBLIC_KEY});
}
