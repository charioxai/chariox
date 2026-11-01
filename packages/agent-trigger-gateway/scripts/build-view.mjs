import { build } from "esbuild"
import { createRequire } from "node:module"
import { fileURLToPath } from "node:url"
const require = createRequire(new URL("../../transcript-view/package.json", import.meta.url))
const markdown = name => fileURLToPath(new URL(`../../transcript-view/src/markdown/${name}`, import.meta.url))
// The renderers load on demand, so each is its own entry beside the viewer, served from the same assets folder.
await build({ entryPoints: { viewer: "src/agent-trigger-viewer.ts", math: require.resolve("katex/dist/katex.min.css"),
    "math-renderer": markdown("math-renderer-chunk.ts"), "code-highlighter": markdown("code-highlighter-chunk.ts") },
  outdir: "dist/agent-trigger-assets", bundle: true,
  splitting: true, format: "esm", target: "es2022", minify: true, loader: { ".woff2": "file", ".woff": "file", ".ttf": "file" },
  define: { __CHARIOX_MATH_STYLESHEET_URL__: JSON.stringify("./assets/math.css"),
    __CHARIOX_MATH_RENDERER_URL__: JSON.stringify("./math-renderer.js"),
    __CHARIOX_CODE_HIGHLIGHTER_URL__: JSON.stringify("./code-highlighter.js") } })
