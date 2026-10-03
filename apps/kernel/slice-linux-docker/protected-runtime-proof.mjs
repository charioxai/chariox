const KERNEL = "/opt/chariox-slice/bin/chariox-kernel"
function refuse() { throw new Error("The protected slice runtime cannot be verified; this slice has not started") }

export function parseRuntimeHash(output) {
  const match = /^([a-f0-9]{64})  \/opt\/chariox-slice\/bin\/chariox-kernel\n?$/.exec(output)
  if (!match) refuse()
  return match[1]
}

export function verifyRuntimeMetadata(output) {
  const paths = ["/", "/opt", "/opt/chariox-slice", "/opt/chariox-slice/bin", KERNEL]
  const rows = output.trimEnd().split("\n")
  if (rows.length !== paths.length) refuse()
  rows.forEach((row, index) => {
    const [uid, mode, type, name] = row.split("|")
    if (uid !== "0" || name !== paths[index] || !/^[0-7]{3,4}$/.test(mode)
        || (Number.parseInt(mode, 8) & 0o022) !== 0
        || type !== (index === paths.length - 1 ? "regular file" : "directory")) refuse()
  })
}

// The managed engine/root administrator is trusted. The ordinary slice user
// must not be able to replace any ancestor or the executable after this check.
export function requireRuntimeProof(docker, container, expectedHash) {
  if (!/^[a-f0-9]{64}$/.test(expectedHash ?? "")) refuse()
  const metadata = docker(["exec", "-u", "0", container, "/usr/bin/stat", "-c", "%u|%a|%F|%n",
    "/", "/opt", "/opt/chariox-slice", "/opt/chariox-slice/bin", KERNEL])
  if (metadata.status !== 0) refuse()
  verifyRuntimeMetadata(String(metadata.stdout))
  const hash = docker(["exec", "-u", "0", container, "/usr/bin/sha256sum", KERNEL])
  if (hash.status !== 0 || parseRuntimeHash(String(hash.stdout)) !== expectedHash) refuse()
}
