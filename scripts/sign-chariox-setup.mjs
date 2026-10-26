#!/usr/bin/env node
// MP-07 / MP-11: owner-side detached signing hook. Builders receive public pins only.
import { spawnSync } from "node:child_process"
import { verify } from "node:crypto"
import { lstat, readFile, writeFile } from "node:fs/promises"
import { resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { publicKeyFromHex } from "./release-bundle.mjs"
export async function signSetup({ artifact, signature, publicKeyHex, signer }) {
  if (!artifact || !signature || !signer || !signer.startsWith("/")) throw new Error("owner signing hook must be an absolute executable path")
  const info = await lstat(artifact)
  if (!info.isFile() || info.isSymbolicLink()) throw new Error("Setup artifact must be regular")
  publicKeyFromHex(publicKeyHex)
  // Hook contract: sign exact final executable bytes, write 128 lowercase hex chars.
  // On macOS codesign/notarize/staple first, then run this detached signature hook.
  if (spawnSync(signer, [resolve(artifact), resolve(signature)], { stdio: "ignore" }).status !== 0) throw new Error("owner signing hook failed")
  const sig = await readFile(signature, "utf8"), bytes = await readFile(artifact)
  if (!/^[a-f0-9]{128}$/.test(sig) || !verify(null, bytes, publicKeyFromHex(publicKeyHex), Buffer.from(sig, "hex"))) throw new Error("Setup signature does not match published public pin")
  return { schema: "chariox.setup-signing.v1", publicKeyHex, verified: true }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const args = process.argv.slice(2), options = {}
  for (let i = 0; i < args.length; i += 2) { if (!/^--(artifact|signature|public-key|signer)$/.test(args[i]) || !args[i + 1]) throw new Error("invalid signing hook arguments"); options[args[i].slice(2)] = args[i + 1] }
  process.stdout.write(JSON.stringify(await signSetup({ ...options, publicKeyHex: options["public-key"] })) + "\n")
}
