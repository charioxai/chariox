import assert from "node:assert/strict"
import { spawnSync } from "node:child_process"
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises"
import { tmpdir } from "node:os"
import path from "node:path"
import test from "node:test"
import { fileURLToPath } from "node:url"

const directory = path.dirname(fileURLToPath(import.meta.url))
const screenScript = path.join(directory, "docker", "slice-screen.sh")
const profile = "/tmp/taskbar-test-profile"

// Every desktop component but the taskbar is running; `status` is the script's own report.
async function screenStatus(root, { taskbarDisabled }) {
  const bin = path.join(root, "bin")
  await mkdir(bin, { recursive: true })
  await writeFile(path.join(bin, "xdpyinfo"), "#!/bin/sh\nexit 0\n")
  await writeFile(
    path.join(bin, "pgrep"),
    `#!/bin/sh
printf '%s\\n' 'Xvfb :99 -screen 0 1280x800x24' 'x11vnc -display :99' 'websockify 0.0.0.0:6080 127.0.0.1:5900' '/usr/bin/chromium --user-data-dir=${profile}' | grep -E -- "$2" | sed 's/^/1 /'
`,
  )
  await chmod(path.join(bin, "xdpyinfo"), 0o755)
  await chmod(path.join(bin, "pgrep"), 0o755)
  const logs = path.join(root, "logs")
  await mkdir(logs, { recursive: true })
  if (taskbarDisabled) await writeFile(path.join(logs, "taskbar.disabled"), "")
  return spawnSync("bash", [screenScript, "status"], {
    encoding: "utf8",
    env: {
      PATH: `${bin}:${process.env.PATH}`,
      HOME: root,
      CHARIOX_SLICE_ROOT: root,
      CHARIOX_SLICE_VIEWER_BACKEND: "novnc",
      CHARIOX_SLICE_CHROME_PROFILE: profile,
    },
  })
}

test("a desktop without a taskbar reports itself available", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "chariox-taskbar-"))
  try {
    const result = await screenStatus(root, { taskbarDisabled: true })
    assert.equal(result.status, 0, result.stdout + result.stderr)
    assert.match(result.stdout, /available=true/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("a desktop that should have a taskbar still reports it missing", async () => {
  const root = await mkdtemp(path.join(tmpdir(), "chariox-taskbar-"))
  try {
    const result = await screenStatus(root, { taskbarDisabled: false })
    assert.equal(result.status, 1)
    assert.match(result.stdout, /missing=taskbar/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test("a Room Environment desktop starts without the taskbar and the others keep it", async () => {
  const script = await readFile(screenScript, "utf8")
  assert.match(script, /if \[\[ -n "\$\{CHARIOX_ROOM_ENVIRONMENT_SESSION_ID:-\}" \]\]; then\n\s+: >"\$TASKBAR_DISABLED"\n\s+else\n\s+rm -f "\$TASKBAR_DISABLED"\n\s+nohup tint2 /)
  assert.match(script, /taskbar_expected && require_process '\(\^\|\/\)tint2/)
})
