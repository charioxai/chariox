import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import { test } from "node:test"

const scriptUrl = new URL("../deploy/managed-kernel/prepare-hetzner-image.sh", import.meta.url)
const versionsUrl = new URL("../deploy/managed-kernel/provider-versions.env", import.meta.url)
const publicationDockerfileUrl = new URL("../docker/publication/Dockerfile", import.meta.url)

test("Hetzner image preparation is pinned, guarded, and leaves no runtime identity", async () => {
  const script = await readFile(scriptUrl, "utf8")

  assert.match(script, /MARKER_VALUE=managed-remote-kernels-image-builder-v1/)
  assert.match(script, /refusing to modify a host that is not marked as the disposable image builder/)
  assert.match(script, /\[ "\$\{ID:-\}" = "ubuntu" \]/)
  assert.match(script, /\[ "\$\{VERSION_ID:-\}" = "26\.04" \]/)
  assert.match(script, /\[ "\$node_major" -eq 22 \]/)
  assert.match(script, /grep -Eq '\^sha256:\[0-9a-f\]\{64\}\$'/)
  assert.match(script, /grep -Eq '\^\[0-9\]\+\\\.\[0-9\]\+\\\.\[0-9\]\+\$'/)
  assert.match(script, /provider-versions\.env/)
  assert.match(script, /"@openai\/codex@\$codex_version"/)
  assert.match(script, /"opencode-ai@\$opencode_version"/)
  assert.match(script, /"@anthropic-ai\/claude-code@\$claude_version"/)
  assert.match(script, /"\$script_root\/install-image\.sh" "\$release_rootfs" "\$release_digest" "\$trusted_public_key"/)
  assert.match(script, /systemctl is-enabled --quiet chariox-managed-bootstrap\.service/)
  assert.match(script, /systemctl is-active --quiet chariox-managed-bootstrap\.service/)
  assert.match(script, /managed runtime state entered the image/)
  assert.match(script, /cloud-init clean --logs --machine-id --seed/)
  assert.match(script, /rm -f \/etc\/ssh\/ssh_host_\*/)
  assert.doesNotMatch(script, /systemctl (?:start|restart|enable --now) chariox-managed-bootstrap/)
  assert.doesNotMatch(script, /\.arroba/)
})

test("Hetzner image preparation installs the hosted-drill tools", async () => {
  const script = await readFile(scriptUrl, "utf8")
  for (const dependency of [
    "bubblewrap",
    "build-essential",
    "ca-certificates",
    "cloud-init",
    "curl",
    "gh",
    "git",
    "jq",
    "nodejs",
    "npm",
    "protobuf-compiler",
    "ripgrep",
    "rsync",
    "socat",
    "unzip",
    "zstd",
  ]) {
    assert.match(script, new RegExp(`\\n  ${dependency.replace(/[.*+?^${}()|[\\]\\]/g, "\\$&")}(?: \\\\|\\n)`))
  }
})

test("managed image and publication runtimes pin the same provider releases", async () => {
  const versions = Object.fromEntries(
    (await readFile(versionsUrl, "utf8"))
      .trim()
      .split("\n")
      .map((line) => line.split("=")),
  )
  const dockerfile = await readFile(publicationDockerfileUrl, "utf8")
  assert.deepEqual(versions, {
    CHARIOX_CODEX_VERSION: "0.144.0",
    CHARIOX_OPENCODE_VERSION: "1.4.1",
    CHARIOX_CLAUDE_VERSION: "2.1.207",
  })
  for (const [name, version] of Object.entries(versions)) {
    assert.match(dockerfile, new RegExp(`ARG ${name}=${version.replaceAll(".", "\\.")}`))
  }
})
