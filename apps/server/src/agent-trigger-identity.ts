import {createECDH} from "node:crypto"
import {closeSync, constants, fstatSync, openSync, readFileSync, writeSync} from "node:fs"

const VERSION = 1

/** The ingress's own relay key, created once by `pair`. Only a private file keeps the kernel's pairing usable. */
export function createTriggerIdentity(path: string): Buffer {
  const ecdh = createECDH("prime256v1"); ecdh.generateKeys()
  // OpenSSL omits leading zero bytes from the scalar; the stored key is fixed-width P-256, as the CLI stores its own.
  const scalar = ecdh.getPrivateKey()
  const key = Buffer.alloc(32)
  scalar.copy(key, key.length - scalar.length)
  const fd = openSync(path, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600)
  try { writeSync(fd, JSON.stringify({version: VERSION, private_key: key.toString("base64url")})) } finally { closeSync(fd) }
  return key
}

export function loadTriggerIdentity(path: string): Buffer | null {
  let fd: number
  try { fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW) } catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error }
  try {
    const stat = fstatSync(fd)
    if (!stat.isFile() || (stat.mode & 0o077) !== 0 || stat.size > 4096) throw Error(`${path} holds the ingress relay key: make it a private regular file (chmod 600)`)
    const stored = JSON.parse(readFileSync(fd, "utf8")) as {version?: number; private_key?: string}
    const key = Buffer.from(stored.private_key ?? "", "base64url")
    if (stored.version !== VERSION || key.length !== 32) throw Error(`${path} is not an ingress relay key: delete it and pair again`)
    return key
  } finally { closeSync(fd) }
}
