import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

const dockerfile = await readFile(new URL("../docker/publication/Dockerfile", import.meta.url), "utf8")
const egressDockerfile = await readFile(new URL("../docker/publication-egress/Dockerfile", import.meta.url), "utf8")
const kernelTypes = await readFile(new URL("../packages/kernel-client/src/kernel-types.ts", import.meta.url), "utf8")
const workflowCode = await readFile(new URL("../apps/kernel/src/workflow_code.rs", import.meta.url), "utf8")

test("publication image copies compile-time workflow examples before building the kernel", () => {
  assert.match(workflowCode, /include_str!\("\.\.\/\.\.\/\.\.\/examples\/workflow-code\//)

  const rustStageStart = dockerfile.indexOf("FROM rust:1.88-bookworm@sha256:")
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

test("publication images pin every base image by immutable digest", () => {
  const publicationBases = dockerfile.match(/^FROM\s+\S+/gm) ?? []
  assert.equal(publicationBases.length, 3)
  for (const base of publicationBases) {
    assert.match(base, /@sha256:[0-9a-f]{64}$/)
  }
  assert.match(egressDockerfile, /^FROM node:22-bookworm-slim@sha256:[0-9a-f]{64}$/m)
})

test("publication image reserves isolated credential, action, and gateway identities", () => {
  assert.match(dockerfile, /useradd --create-home --uid 1001 .* chariox/)
  assert.match(dockerfile, /useradd --create-home --uid 1002 .* chariox-action/)
  assert.match(dockerfile, /useradd --create-home --uid 1003 .* chariox-gateway/)
  assert.match(dockerfile, /chown -R root:root \/opt\/chariox/)
  assert.match(dockerfile, /chmod -R go-w \/opt\/chariox/)
  assert.doesNotMatch(dockerfile, /chown -R chariox:chariox \/opt\/chariox/)
  assert.match(dockerfile, /chmod 700 \/home\/chariox \/home\/chariox-action \/home\/chariox-gateway/)
  assert.match(dockerfile, /WORKDIR \/workspace/)
  assert.match(dockerfile, /ENTRYPOINT \["tini", "--", "chariox-publication-container"\]/)
  assert.doesNotMatch(dockerfile, /^USER\s+/m, "PID 1 must retain only the root bootstrap needed to prepare isolated role state")
})

test("publication image pins and verifies every official provider CLI", () => {
  assert.match(dockerfile, /ARG CHARIOX_CODEX_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /ARG CHARIOX_OPENCODE_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /ARG CHARIOX_CLAUDE_VERSION=\d+\.\d+\.\d+/)
  assert.match(dockerfile, /"@openai\/codex@\$\{CHARIOX_CODEX_VERSION\}"/)
  assert.match(dockerfile, /"opencode-ai@\$\{CHARIOX_OPENCODE_VERSION\}"/)
  assert.match(dockerfile, /"@anthropic-ai\/claude-code@\$\{CHARIOX_CLAUDE_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(codex --version\)" = "codex-cli \$\{CHARIOX_CODEX_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(opencode --version\)" = "\$\{CHARIOX_OPENCODE_VERSION\}"/)
  assert.match(dockerfile, /test "\$\(claude --version\)" = "\$\{CHARIOX_CLAUDE_VERSION\} \(Claude Code\)"/)
  assert.doesNotMatch(dockerfile, /npm install -g\s+@openai\/codex(?:\s|$)/)
  assert.doesNotMatch(dockerfile, /npm install -g\s+opencode-ai(?:\s|$)/)
  assert.doesNotMatch(dockerfile, /npm install -g\s+@anthropic-ai\/claude-code(?:\s|$)/)
})

test("publication image labels the protocol version verified against its kernel", () => {
  const protocolVersion = kernelTypes.match(/LOCAL_DAEMON_PROTOCOL_VERSION\s*=\s*(\d+)/)?.[1]
  assert.ok(protocolVersion, "the shared kernel client protocol version must be readable")
  assert.match(
    dockerfile,
    new RegExp(`ARG CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION=${protocolVersion}`, "g"),
  )
  assert.match(
    dockerfile,
    /chariox-kernel --print-local-daemon-protocol-version\)" = "\$\{CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION\}"/,
  )
  assert.match(
    dockerfile,
    /LABEL dev\.chariox\.local-daemon-protocol-version="\$\{CHARIOX_LOCAL_DAEMON_PROTOCOL_VERSION\}"/,
  )
})

test("publication egress image runs only the dedicated unprivileged gateway", () => {
  assert.match(egressDockerfile, /USER 10001:10001/)
  assert.match(egressDockerfile, /ENTRYPOINT \["node", "\/opt\/chariox-egress\/gateway\.mjs"\]/)
  assert.doesNotMatch(egressDockerfile, /COPY apps|COPY packages|COPY \. \/|npm install/)
})
