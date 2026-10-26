import { writeSync } from "node:fs"

import type { TerminalAttachOutput } from "./room-terminal-attach.js"

const PAUSE = new Int32Array(new SharedArrayBuffer(4))

/**
 * stdout written straight to the file descriptor. The TUI renderer replaces `process.stdout.write`
 * and swallows what an attached terminal draws, so an attach must not go through it.
 */
export function directTerminalOutput(stream: NodeJS.WriteStream = process.stdout, fd = 1): TerminalAttachOutput {
  return {
    get columns() { return stream.columns },
    get rows() { return stream.rows },
    write(data) {
      const bytes = typeof data === "string" ? Buffer.from(data) : data
      for (let written = 0; written < bytes.length;) {
        try {
          written += writeSync(fd, bytes, written)
        } catch (error) {
          if ((error as NodeJS.ErrnoException).code !== "EAGAIN") throw error
          Atomics.wait(PAUSE, 0, 0, 2)
        }
      }
      return true
    },
    on: (event, listener) => stream.on(event, listener),
    off: (event, listener) => stream.off(event, listener),
  }
}
