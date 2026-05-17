import assert from "node:assert/strict"
import { mkdir, writeFile } from "node:fs/promises"
import os from "node:os"
import path from "node:path"
import test from "node:test"

import { themeDirectories, themeFiles } from "./theme-file-source.js"

test("themeDirectories uses XDG config and workspace theme locations", () => {
  const previous = process.env.XDG_CONFIG_HOME
  try {
    process.env.XDG_CONFIG_HOME = "/tmp/arroba-config"
    assert.deepEqual(themeDirectories("/workspace/project"), [
      "/tmp/arroba-config/arroba/themes",
      "/workspace/project/.arroba/themes",
    ])
  } finally {
    if (previous === undefined) {
      delete process.env.XDG_CONFIG_HOME
    } else {
      process.env.XDG_CONFIG_HOME = previous
    }
  }
})

test("themeFiles returns sorted json files only", async () => {
  const root = path.join(os.tmpdir(), `arroba-theme-source-${Date.now()}`)
  await mkdir(root, { recursive: true })
  await writeFile(path.join(root, "b.json"), "{}")
  await writeFile(path.join(root, "a.json"), "{}")
  await writeFile(path.join(root, "notes.txt"), "")

  assert.deepEqual(await themeFiles(root), [
    path.join(root, "a.json"),
    path.join(root, "b.json"),
  ])
})
