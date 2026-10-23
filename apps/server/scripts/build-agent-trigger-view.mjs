import { build } from "esbuild"
import { createRequire } from "node:module"
const require = createRequire(new URL("../../../packages/transcript-view/package.json", import.meta.url))
await build({ entryPoints: { viewer: "src/agent-trigger-viewer.ts", math: require.resolve("katex/dist/katex.min.css") }, outdir: "dist/agent-trigger-assets", bundle: true,
  splitting: true, format: "esm", target: "es2022", minify: true, loader: { ".woff2": "file", ".woff": "file", ".ttf": "file" },
  define: { __CHARIOX_MATH_STYLESHEET_URL__: JSON.stringify("./assets/math.css") } })
