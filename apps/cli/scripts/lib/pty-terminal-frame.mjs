import { spawn } from "node:child_process"
import { mkdir, rm, writeFile } from "node:fs/promises"
import path from "node:path"

const ESC = "\u001b"

export function replayPtyFrame(raw, columns, rows) {
  const width = Math.max(1, Math.floor(columns))
  const height = Math.max(1, Math.floor(rows))
  const blankRow = () => Array.from({ length: width }, () => " ")
  let screen = Array.from({ length: height }, blankRow)
  let row = 0
  let column = 0
  let savedRow = 0
  let savedColumn = 0

  const clampCursor = () => {
    row = Math.max(0, Math.min(height - 1, row))
    column = Math.max(0, Math.min(width - 1, column))
  }
  const clear = () => {
    screen = Array.from({ length: height }, blankRow)
    row = 0
    column = 0
  }
  const scrollUp = (count = 1) => {
    for (let index = 0; index < count; index += 1) {
      screen.shift()
      screen.push(blankRow())
    }
  }
  const scrollDown = (count = 1) => {
    for (let index = 0; index < count; index += 1) {
      screen.pop()
      screen.unshift(blankRow())
    }
  }
  const lineFeed = () => {
    row += 1
    if (row >= height) {
      scrollUp()
      row = height - 1
    }
  }
  const parseParams = (body) => {
    const normalized = body.replace(/^[?>!]/, "")
    return normalized.split(";").map((value) => value === "" ? 0 : Number.parseInt(value, 10) || 0)
  }
  const applyCsi = (body, command) => {
    const params = parseParams(body)
    const first = params[0] || 1
    if ((command === "h" || command === "l") && body.includes("1049") && command === "h") {
      clear()
      return
    }
    switch (command) {
      case "A": row -= first; break
      case "B": row += first; break
      case "C": column += first; break
      case "D": column -= first; break
      case "E": row += first; column = 0; break
      case "F": row -= first; column = 0; break
      case "G": column = first - 1; break
      case "d": row = first - 1; break
      case "H":
      case "f": row = (params[0] || 1) - 1; column = (params[1] || 1) - 1; break
      case "J": {
        const mode = params[0] || 0
        if (mode === 2 || mode === 3) clear()
        else if (mode === 0) {
          for (let target = column; target < width; target += 1) screen[row][target] = " "
          for (let targetRow = row + 1; targetRow < height; targetRow += 1) screen[targetRow] = blankRow()
        } else if (mode === 1) {
          for (let targetRow = 0; targetRow < row; targetRow += 1) screen[targetRow] = blankRow()
          for (let target = 0; target <= column; target += 1) screen[row][target] = " "
        }
        break
      }
      case "K": {
        const mode = params[0] || 0
        const start = mode === 0 ? column : 0
        const end = mode === 1 ? column + 1 : width
        for (let target = start; target < end; target += 1) screen[row][target] = " "
        break
      }
      case "S": scrollUp(first); break
      case "T": scrollDown(first); break
      case "@": screen[row].splice(column, 0, ...Array.from({ length: first }, () => " ")); screen[row].length = width; break
      case "P": screen[row].splice(column, first); while (screen[row].length < width) screen[row].push(" "); break
      case "X": for (let target = column; target < Math.min(width, column + first); target += 1) screen[row][target] = " "; break
      case "s": savedRow = row; savedColumn = column; break
      case "u": row = savedRow; column = savedColumn; break
      default: break
    }
    clampCursor()
  }

  for (let index = 0; index < raw.length;) {
    const character = raw[index]
    if (character === ESC) {
      const next = raw[index + 1]
      if (next === "[") {
        let end = index + 2
        while (end < raw.length && !/[\x40-\x7e]/.test(raw[end])) end += 1
        if (end >= raw.length) break
        applyCsi(raw.slice(index + 2, end), raw[end])
        index = end + 1
        continue
      }
      if (next === "]" || next === "P" || next === "_" || next === "^") {
        let end = index + 2
        while (end < raw.length && raw[end] !== "\u0007" && !(raw[end] === ESC && raw[end + 1] === "\\")) end += 1
        index = raw[end] === "\u0007" ? end + 1 : Math.min(raw.length, end + 2)
        continue
      }
      if (next === "7") { savedRow = row; savedColumn = column; index += 2; continue }
      if (next === "8") { row = savedRow; column = savedColumn; clampCursor(); index += 2; continue }
      index += next === "(" || next === ")" ? 3 : 2
      continue
    }
    if (character === "\r") { column = 0; index += 1; continue }
    if (character === "\n") { lineFeed(); index += 1; continue }
    if (character === "\b") { column = Math.max(0, column - 1); index += 1; continue }
    if (character === "\t") { column = Math.min(width - 1, (Math.floor(column / 8) + 1) * 8); index += 1; continue }
    const codePoint = raw.codePointAt(index)
    const text = String.fromCodePoint(codePoint)
    index += text.length
    if (codePoint < 0x20 || codePoint === 0x7f) continue
    screen[row][column] = text
    column += 1
    if (column >= width) {
      column = 0
      lineFeed()
    }
  }

  const lines = screen.map((cells) => cells.join("").replace(/\s+$/u, ""))
  return {
    columns: width,
    rows: height,
    cursor: { row, column },
    lines,
    text: lines.join("\n"),
  }
}

export function stripPtyControlSequences(raw) {
  return raw
    .replace(/\u001b\][^\u0007]*(?:\u0007|\u001b\\)/g, "")
    .replace(/\u001bP[\s\S]*?\u001b\\/g, "")
    .replace(/\u001b\[[0-?]*[ -/]*[@-~]/g, "")
    .replace(/\u001b[()][0-2A-Z]/g, "")
    .replace(/\u001b./g, "")
    .replace(/\r/g, "")
}

export async function writePtyFrameArtifacts({
  raw,
  outputDir,
  name,
  columns,
  rows,
  screenshotColumns = columns,
  screenshotRows = rows,
}) {
  await mkdir(outputDir, { recursive: true })
  const frame = replayPtyFrame(raw, columns, rows)
  const textPath = path.join(outputDir, `${name}.txt`)
  const svgPath = path.join(outputDir, `${name}.svg`)
  const pngPath = path.join(outputDir, `${name}.png`)
  await writeFile(textPath, `${frame.text}\n`, "utf8")
  const fontSize = 14
  const lineHeight = 19
  const characterWidth = 8.5
  const padding = 14
  const visibleColumns = Math.max(1, Math.min(frame.columns, Math.floor(screenshotColumns)))
  const visibleRows = Math.max(1, Math.min(frame.rows, Math.floor(screenshotRows)))
  const visibleLines = frame.lines.slice(0, visibleRows).map((line) => Array.from(line).slice(0, visibleColumns).join(""))
  const width = Math.ceil(visibleColumns * characterWidth + padding * 2)
  const height = Math.ceil(visibleRows * lineHeight + padding * 2)
  const escapedLines = visibleLines.map((line, index) => (
    `<text x="${padding}" y="${padding + fontSize + index * lineHeight}">${escapeXml(line || " ")}</text>`
  )).join("\n")
  await writeFile(svgPath, `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}">
<rect width="100%" height="100%" fill="#0b1016"/>
<g fill="#e7edf3" font-family="Menlo, Monaco, Consolas, monospace" font-size="${fontSize}" xml:space="preserve">${escapedLines}</g>
</svg>\n`, "utf8")
  const converted = await run("sips", ["-s", "format", "png", svgPath, "--out", pngPath])
  if (converted.code !== 0) {
    throw new Error(`failed to rasterize captured PTY frame ${name}: ${converted.stderr || converted.stdout}`)
  }
  await rm(svgPath, { force: true })
  return {
    frame,
    screenshot: { columns: visibleColumns, rows: visibleRows },
    textPath,
    pngPath,
  }
}

function run(command, args) {
  return new Promise((resolve, reject) => {
    const child = spawn(command, args, { stdio: ["ignore", "pipe", "pipe"] })
    let stdout = ""
    let stderr = ""
    child.stdout.on("data", (chunk) => { stdout += chunk.toString() })
    child.stderr.on("data", (chunk) => { stderr += chunk.toString() })
    child.once("error", reject)
    child.once("close", (code) => resolve({ code, stdout, stderr }))
  })
}

function escapeXml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
}
