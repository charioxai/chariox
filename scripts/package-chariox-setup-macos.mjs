#!/usr/bin/env node
// MP-07 / MP-11: unsigned app skeleton; Developer ID + notarization are owner-only.
import { copyFile, mkdir, writeFile } from "node:fs/promises"
import { join } from "node:path"
const [binary, output] = process.argv.slice(2)
if (!binary || !output) throw new Error("usage: package-chariox-setup-macos BINARY OUTPUT.app")
const contents = join(output, "Contents"), macos = join(contents, "MacOS")
await mkdir(macos, { recursive: true })
await copyFile(binary, join(macos, "chariox-setup"))
await writeFile(join(contents, "Info.plist"), `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.chariox.setup</string><key>CFBundleName</key><string>Chariox Setup</string><key>CFBundleExecutable</key><string>Chariox Setup</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>\n`)
// Generic app opens its terminal-based setup flow; no user-specific secret or sudo.
await writeFile(join(macos, "Chariox Setup"), `#!/bin/sh\nset -eu\nroot=$(cd -- "$(dirname -- "$0")" && pwd)\nexec /usr/bin/osascript - "$root/chariox-setup" <<'APPLESCRIPT'\non run argv\n  tell application "Terminal"\n    activate\n    do script (quoted form of item 1 of argv)\n  end tell\nend run\nAPPLESCRIPT\n`, { mode: 0o755 })
