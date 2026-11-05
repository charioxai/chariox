# Publish only this installer's own links, including their first installation.
publish_app_release_link() {
  node - "$1" "$2" <<'NODE'
const fs = require("node:fs")
const path = require("node:path")
const [target, destination] = process.argv.slice(2)
const parent = fs.lstatSync(path.dirname(destination))
if (!parent.isDirectory() || parent.isSymbolicLink() || parent.uid !== 0 || (parent.mode & 0o022)) throw new Error("unsafe App link parent")
try {
  const previous = fs.lstatSync(destination)
  if (!previous.isSymbolicLink() || fs.readlinkSync(destination) !== target || previous.uid !== 0) throw new Error("managed App release link is obstructed")
} catch (error) { if (error.code !== "ENOENT") throw error }
const temporary = `${destination}.app-new-${process.pid}`
try {
  fs.symlinkSync(target, temporary)
  fs.renameSync(temporary, destination)
  const directory = fs.openSync(path.dirname(destination), fs.constants.O_RDONLY | fs.constants.O_DIRECTORY | fs.constants.O_NOFOLLOW)
  try { fs.fsyncSync(directory) } finally { fs.closeSync(directory) }
} finally { try { fs.unlinkSync(temporary) } catch (error) { if (error.code !== "ENOENT") throw error } }
NODE
}
