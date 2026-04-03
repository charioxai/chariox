import { mkdir, readdir, readFile, rm, writeFile } from "node:fs/promises"
import path from "node:path"
import { fileURLToPath } from "node:url"

import { transformAsync } from "@babel/core"
import tsPreset from "@babel/preset-typescript"
import solidPreset from "babel-preset-solid"

const __dirname = path.dirname(fileURLToPath(import.meta.url))
const appDir = path.resolve(__dirname, "..")
const srcDir = path.join(appDir, "src")
const distDir = path.join(appDir, "dist")

await rm(distDir, { force: true, recursive: true })
await mkdir(distDir, { recursive: true })

for (const sourcePath of await collectSourceFiles(srcDir)) {
  const outputPath = path.join(
    distDir,
    path.relative(srcDir, sourcePath).replace(/\.tsx?$/, ".js"),
  )
  await mkdir(path.dirname(outputPath), { recursive: true })
  const code = await readFile(sourcePath, "utf8")
  const transformed = await transformAsync(code, {
    filename: sourcePath,
    presets: [
      [
        solidPreset,
        {
          moduleName: "@opentui/solid",
          generate: "universal",
        },
      ],
      [tsPreset],
    ],
    sourceMaps: true,
  })

  await writeFile(outputPath, transformed?.code ?? "", "utf8")
}

async function collectSourceFiles(directory) {
  const files = []
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const entryPath = path.join(directory, entry.name)
    if (entry.isDirectory()) {
      files.push(...await collectSourceFiles(entryPath))
      continue
    }
    if (!entry.isFile()) {
      continue
    }
    if (!entry.name.endsWith(".ts") && !entry.name.endsWith(".tsx")) {
      continue
    }
    files.push(entryPath)
  }
  return files
}
