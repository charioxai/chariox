import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

const dockerfile = await readFile(new URL("../docker/publication/Dockerfile", import.meta.url), "utf8")
const egressDockerfile = await readFile(new URL("../docker/publication-egress/Dockerfile", import.meta.url), "utf8")
const workflowCode = await readFile(new URL("../apps/kernel/src/workflow_code.rs", import.meta.url), "utf8")

test("publication image copies compile-time workflow examples before building the kernel", () => {
  assert.match(workflowCode, /include_str!\("\.\.\/\.\.\/\.\.\/examples\/workflow-code\//)

  const rustStageStart = dockerfile.indexOf("FROM rust:1.88-bookworm AS rust-builder")
  const nextStageStart = dockerfile.indexOf("\nFROM ", rustStageStart + 1)
  assert.notEqual(rustStageStart, -1)
  assert.notEqual(nextStageStart, -1)

  const rustStage = dockerfile.slice(rustStageStart, nextStageStart)
  const examplesCopy = rustStage.indexOf("COPY examples/workflow-code examples/workflow-code")
  const kernelBuild = rustStage.indexOf("RUN cargo build --manifest-path apps/kernel/Cargo.toml")
  assert.ok(examplesCopy >= 0, "the Rust build stage must copy compile-time workflow examples")
  assert.ok(kernelBuild >= 0, "the Rust build stage must compile the kernel")
  assert.ok(examplesCopy < kernelBuild, "compile-time workflow examples must be copied before the kernel build")
})

test("publication image reserves isolated credential, action, and gateway identities", () => {
  assert.match(dockerfile, /useradd --create-home --uid 1001 .* arroba/)
  assert.match(dockerfile, /useradd --create-home --uid 1002 .* arroba-action/)
  assert.match(dockerfile, /useradd --create-home --uid 1003 .* arroba-gateway/)
  assert.match(dockerfile, /chown -R root:root \/opt\/arroba/)
  assert.match(dockerfile, /chmod -R go-w \/opt\/arroba/)
  assert.doesNotMatch(dockerfile, /chown -R arroba:arroba \/opt\/arroba/)
  assert.match(dockerfile, /chmod 700 \/home\/arroba \/home\/arroba-action \/home\/arroba-gateway/)
  assert.match(dockerfile, /WORKDIR \/workspace/)
  assert.match(dockerfile, /ENTRYPOINT \["tini", "--", "arroba-publication-container"\]/)
  assert.doesNotMatch(dockerfile, /^USER\s+/m, "PID 1 must retain only the root bootstrap needed to prepare isolated role state")
})

test("publication image pins and verifies every official provider CLI", () => {
  assert.match(dockerfile, /ARG ARROBA_CODEX_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /ARG ARROBA_OPENCODE_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /ARG ARROBA_CLAUDE_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /"@openai\/codex@\$\{ARROBA_CODEX_VERSION\}"/)
  assert.match(dockerfile, /"opencode-ai@\$\{ARROBA_OPENCODE_VERSION\}"/)
  assert.match(dockerfile, /"@anthropic-ai\/claude-code@\$\{ARROBA_CLAUDE_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(codex --version\)" = "codex-cli \$\{ARROBA_CODEX_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(opencode --version\)" = "\$\{ARROBA_OPENCODE_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(claude --version\)" = "\$\{ARROBA_CLAUDE_VERSION\} \(Claude Code\)"/)
  assert.doesNotMatch(dockerfile, /npm install -g\s+@openai\/codex(?:\s|$)/)
  assert.doesNotMatch(dockerfile, /npm install -g\s+opencode-ai(?:\s|$)/)
  assert.doesNotMatch(dockerfile, /npm install -g\s+@anthropic-ai\/claude-code(?:\s|$)/)
})

test("publication egress image runs only the dedicated unprivileged gateway", () => {
  assert.match(egressDockerfile, /USER 10001:10001/)
  assert.match(egressDockerfile, /ENTRYPOINT \["node", "\/opt\/arroba-egress\/gateway\.mjs"\]/)
  assert.doesNotMatch(egressDockerfile, /COPY apps|COPY packages|COPY \. \/|npm install/)
})
