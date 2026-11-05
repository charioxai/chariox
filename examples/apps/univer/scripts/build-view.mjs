import { build } from 'esbuild';
import { cp, mkdir, readFile, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { root, checkLicenses, notices } from './licenses.mjs';

await checkLicenses();
const output = path.resolve(process.argv[2] ?? path.join(root, 'bundle'));
await mkdir(path.join(output, 'ui'), { recursive: true });
const built = await build({
  absWorkingDir: root,
  entryPoints: [path.join(root, 'view/main.mjs')], outfile: path.join(output, 'ui/app.js'),
  bundle: true, format: 'esm', platform: 'browser', target: 'es2022', minify: true,
  metafile: true, legalComments: 'eof', define: { 'process.env.NODE_ENV': '"production"' },
  // This English reference App does not ship Docs' optional language
  // dictionaries. Keep the upstream English loader and its normal support
  // detection, rather than leaving broken lazy imports in the package.
  plugins: [{ name: 'english-document-hyphenation', setup(builder) {
    builder.onLoad({ filter: /@univerjs\/engine-render\/lib\/es\/index\.js$/ }, async ({ path: source }) => {
      const text = await readFile(source, 'utf8');
      const loaders = text.match(/const PATTERN_LOADERS = \{[\s\S]*?\n\};/g);
      if (loaders?.length !== 1) throw new Error('Review the pinned Univer document dictionary loader');
      const english = loaders[0].split('\n').filter(line => line.includes('["en-us"]'));
      if (english.length !== 1) throw new Error('Pinned Univer English dictionary is missing');
      return { contents: text.replace(loaders[0], `const PATTERN_LOADERS = {\n${english[0]}\n};`), loader: 'js' };
    });
  } }],
  loader: { '.woff': 'file', '.woff2': 'file', '.ttf': 'file', '.svg': 'file', '.png': 'file' },
});
for (const name of Object.keys(built.metafile.inputs)) {
  if (/@univerjs-pro[+/]|univer-pro/i.test(name)) throw new Error(`Pro code forbidden in bundle: ${name}`);
  if (!['view/', 'schemas/', 'node_modules/.pnpm/'].some(prefix => name.startsWith(prefix))) throw new Error(`Unlisted bundle input: ${name}`);
}
const viewBytes = Object.values(built.metafile.outputs).reduce((sum, file) => sum + file.bytes, 0);
if (viewBytes + 1024 > 8 * 1024 * 1024) throw new Error(`View exceeds the 8 MiB App asset budget (${viewBytes} bytes)`);
await cp(path.join(root, 'view/index.html'), path.join(output, 'ui/index.html'));
await cp(path.join(root, 'schemas'), path.join(output, 'schemas'), { recursive: true });
await writeFile(path.join(output, 'THIRD_PARTY_NOTICES.txt'), await notices());
await cp(path.join(root, 'licenses.json'), path.join(output, 'DEPENDENCIES.json'));
await writeFile(path.join(output, 'UNIVER_MODIFICATIONS.txt'), 'Univer 1.2.0 engine-render/lib/es/index.js is modified at build time: PATTERN_LOADERS retains only en-us. This App ships English document hyphenation only. Other upstream source and Apache-2.0 notices are retained.\n');
const metadata = { ...built.metafile, outputs: Object.fromEntries(Object.entries(built.metafile.outputs)
  .map(([name, data]) => [path.relative(output, path.resolve(root, name)), data])) };
await writeFile(path.join(output, 'ui-build.json'), JSON.stringify(metadata, null, 2) + '\n');
console.log(`Built Univer view at ${output}; ${Object.keys(built.metafile.inputs).length} admitted inputs`);
