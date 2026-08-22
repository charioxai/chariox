#!/usr/bin/env node

import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto"
import { constants, createReadStream } from "node:fs"
import { chmod, copyFile, lstat, mkdir, readFile, readdir, utimes, writeFile } from "node:fs/promises"
import { basename, dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex")
const REQUIRED_OPTIONS = ["kernel", "supervisor", "signing-key", "output"]

function usage() {
  return "usage: package-managed-kernel-release --kernel <path> --supervisor <path> --signing-key <path> --output <directory>"
}

function parseOptions(argv) {
  const options = new Map()
  for (let index = 0; index < argv.length; index += 2) {
    const option = argv[index]
    const value = argv[index + 1]
    if (!option?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error(usage())
    }
    const name = option.slice(2)
    if (!REQUIRED_OPTIONS.includes(name) || options.has(name)) {
      throw new Error(usage())
    }
    options.set(name, resolve(value))
  }
  if (options.size !== REQUIRED_OPTIONS.length) {
    throw new Error(usage())
  }
  return Object.fromEntries(options)
}

async function requireRegularFile(path, label, maxBytes = Number.MAX_SAFE_INTEGER) {
  const metadata = await lstat(path).catch((error) => {
    throw new Error(`${label} cannot be read: ${error.message}`)
  })
  if (metadata.isSymbolicLink() || !metadata.isFile() || metadata.size > maxBytes) {
    throw new Error(`${label} must be a bounded regular file`)
  }
  return metadata
}

async function prepareOutput(path) {
  const metadata = await lstat(path).catch((error) => {
    if (error.code === "ENOENT") return null
    throw error
  })
  if (metadata) {
    if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
      throw new Error("output must be a directory, not a symlink")
    }
    if ((await readdir(path)).length !== 0) {
      throw new Error("output directory must be empty")
    }
  } else {
    await mkdir(path, { recursive: true, mode: 0o755 })
  }
}

async function sha256File(path) {
  const hash = createHash("sha256")
  for await (const chunk of createReadStream(path)) hash.update(chunk)
  return `sha256:${hash.digest("hex")}`
}

async function installFile(source, destination, mode) {
  await mkdir(dirname(destination), { recursive: true, mode: 0o755 })
  await copyFile(source, destination, constants.COPYFILE_EXCL)
  await chmod(destination, mode)
}

function sourceDateEpoch() {
  const value = process.env.SOURCE_DATE_EPOCH ?? "0"
  if (!/^(?:0|[1-9][0-9]*)$/.test(value) || !Number.isSafeInteger(Number(value))) {
    throw new Error("SOURCE_DATE_EPOCH must be a non-negative integer")
  }
  return new Date(Number(value) * 1000)
}

async function normalizeTree(root, timestamp) {
  const entries = await readdir(root, { withFileTypes: true })
  for (const entry of entries) {
    const path = join(root, entry.name)
    if (entry.isDirectory()) {
      await normalizeTree(path, timestamp)
      await chmod(path, 0o755)
    } else if (entry.isFile()) {
      const executable =
        path.endsWith("/usr/local/bin/chariox-kernel") ||
        path.endsWith("/usr/local/bin/chariox-managed-bootstrap")
      await chmod(path, executable ? 0o755 : 0o644)
    } else {
      throw new Error("packaged release contains an unsupported file type")
    }
    await utimes(path, timestamp, timestamp)
  }
  await chmod(root, 0o755)
  await utimes(root, timestamp, timestamp)
}

function rawEd25519PublicKey(privateKey) {
  const publicDer = createPublicKey(privateKey).export({ format: "der", type: "spki" })
  if (
    publicDer.length !== ED25519_SPKI_PREFIX.length + 32 ||
    !publicDer.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)
  ) {
    throw new Error("signing key did not produce the expected Ed25519 public key")
  }
  return publicDer.subarray(ED25519_SPKI_PREFIX.length)
}

async function packageRelease(options) {
  await requireRegularFile(options.kernel, "kernel binary")
  await requireRegularFile(options.supervisor, "bootstrap supervisor")
  const signingKeyMetadata = await requireRegularFile(
    options["signing-key"],
    "release signing key",
    64 * 1024,
  )
  if (process.platform !== "win32" && (signingKeyMetadata.mode & 0o077) !== 0) {
    throw new Error("release signing key must not be readable by group or other users")
  }
  await prepareOutput(options.output)

  const rootfs = join(options.output, "rootfs")
  const kernelDestination = join(rootfs, "usr/local/bin/chariox-kernel")
  const supervisorDestination = join(rootfs, "usr/local/bin/chariox-managed-bootstrap")
  const releaseDirectory = join(rootfs, "usr/lib/chariox")
  const serviceDestination = join(
    rootfs,
    "etc/systemd/system/chariox-managed-bootstrap.service",
  )
  const serviceSource = fileURLToPath(
    new URL("../deploy/managed-kernel/chariox-managed-bootstrap.service", import.meta.url),
  )
  await requireRegularFile(serviceSource, "managed bootstrap service", 64 * 1024)

  await installFile(options.kernel, kernelDestination, 0o755)
  await installFile(options.supervisor, supervisorDestination, 0o755)
  await installFile(serviceSource, serviceDestination, 0o644)
  const sourceKernelDigest = await sha256File(options.kernel)
  const packagedKernelDigest = await sha256File(kernelDestination)
  if (sourceKernelDigest !== packagedKernelDigest) {
    throw new Error("kernel binary changed while the release was packaged")
  }
  const sourceSupervisorDigest = await sha256File(options.supervisor)
  const packagedSupervisorDigest = await sha256File(supervisorDestination)
  if (sourceSupervisorDigest !== packagedSupervisorDigest) {
    throw new Error("bootstrap supervisor changed while the release was packaged")
  }
  const sourceServiceDigest = await sha256File(serviceSource)
  const packagedServiceDigest = await sha256File(serviceDestination)
  if (sourceServiceDigest !== packagedServiceDigest) {
    throw new Error("managed bootstrap service changed while the release was packaged")
  }

  const manifestBytes = Buffer.from(
    JSON.stringify({
      schemaVersion: 1,
      artifacts: [
        {
          name: "chariox-kernel",
          path: "/usr/local/bin/chariox-kernel",
          sha256: packagedKernelDigest,
        },
        {
          name: "chariox-managed-bootstrap",
          path: "/usr/local/bin/chariox-managed-bootstrap",
          sha256: packagedSupervisorDigest,
        },
        {
          name: "chariox-managed-bootstrap.service",
          path: "/etc/systemd/system/chariox-managed-bootstrap.service",
          sha256: packagedServiceDigest,
        },
      ],
    }),
  )
  const privateKey = createPrivateKey({
    key: await readFile(options["signing-key"]),
    format: "pem",
    type: "pkcs8",
  })
  if (privateKey.asymmetricKeyType !== "ed25519") {
    throw new Error("release signing key must be an Ed25519 PKCS8 private key")
  }
  const signature = sign(null, manifestBytes, privateKey)
  if (signature.length !== 64) {
    throw new Error("release signature has the wrong length")
  }

  await mkdir(releaseDirectory, { recursive: true, mode: 0o755 })
  await writeFile(join(releaseDirectory, "release-manifest.json"), manifestBytes, {
    flag: "wx",
    mode: 0o644,
  })
  await writeFile(join(releaseDirectory, "release-manifest.sig"), signature.toString("base64"), {
    flag: "wx",
    mode: 0o644,
  })
  await writeFile(
    join(releaseDirectory, "release-public-key"),
    rawEd25519PublicKey(privateKey).toString("base64"),
    { flag: "wx", mode: 0o644 },
  )
  await normalizeTree(rootfs, sourceDateEpoch())
  const releaseDigest = `sha256:${createHash("sha256").update(manifestBytes).digest("hex")}`
  process.stdout.write(`${releaseDigest}\n`)
}

try {
  await packageRelease(parseOptions(process.argv.slice(2)))
} catch (error) {
  const message = error instanceof Error ? error.message : String(error)
  process.stderr.write(`${basename(process.argv[1])}: ${message}\n`)
  process.exitCode = 1
}
