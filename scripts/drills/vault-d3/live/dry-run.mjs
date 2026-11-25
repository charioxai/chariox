import { startDrillKernel } from "./drill-kernel.mjs"
import { execFileSync } from "node:child_process"
const k = await startDrillKernel({ label: "d3dry", binary: process.env.D3_KERNEL_BIN, stateRoot: "/root/.chariox/dev/orch15-vaultd3prep/state", evidence: process.env.D3_EVIDENCE, tokenFile: "/root/.claude-lanes/oauth-token.chariox" })
try {
  const r = await k.room("d3dry", k.dir + "/work/dry")
  const cards = await k.ask(r, "Reply READY as your final answer. Do not use tools.", { timeoutMs: 180000 })
  k.note("asked", { cards: cards.length })
  console.log(execFileSync("find", [k.dir, "-maxdepth", "3", "-not", "-path", "*/node_modules*", "-not", "-path", "*/home/.claude/*"], { encoding: "utf8" }).split("\n").slice(0, 80).join("\n"))
} finally { await k.stop() }
