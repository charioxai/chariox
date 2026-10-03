# Shared root-owned App storage enrollment for managed install and upgrade.
enroll_managed_app_storage() (
  set -eu
  install_root=$1
  app_storage_service=$2
# Root-installed authority is derived from the actual managed kernel OS user.
# App requests cannot supply this UID/GID, cgroup root, quota, or filesystem path.
app_storage_uid=$(id -u chariox)
app_storage_gid=$(id -g chariox)
for app_storage_id in "$app_storage_uid" "$app_storage_gid"; do
  case "$app_storage_id" in
    ''|*[!0-9]*|0) echo "invalid managed App storage owner" >&2; exit 1 ;;
  esac
done
for app_storage_path in "$install_root/etc/chariox" "$install_root/var/lib/chariox-app-storage"; do
  if [ -L "$app_storage_path" ] || { [ -e "$app_storage_path" ] && [ ! -d "$app_storage_path" ]; }; then
    echo "managed App storage root is obstructed" >&2; exit 1
  fi
done
install -d -o root -g root -m 0755 "$install_root/etc/chariox"
install -d -o root -g root -m 0711 "$install_root/var/lib/chariox-app-storage"
app_storage_config=$install_root/etc/chariox/app-storage.json
if [ -L "$app_storage_config" ] || { [ -e "$app_storage_config" ] && [ ! -f "$app_storage_config" ]; }; then
  echo "managed App storage enrollment is obstructed" >&2; exit 1
fi
app_storage_pending=$(mktemp "$install_root/etc/chariox/.app-storage.XXXXXXXX")
printf '{"schema":"chariox.app-storage-enrollment.v1","owners":[{"uid":%s,"gid":%s,"cgroup_root":"/sys/fs/cgroup/system.slice/%s/apps","kernel_database_paths":["/home/chariox/.chariox/state/kernel.db"]}]}\n' "$app_storage_uid" "$app_storage_gid" "$app_storage_service" > "$app_storage_pending"
chmod 0644 "$app_storage_pending"
chown root:root "$app_storage_pending"
if [ -e "$app_storage_config" ] && ! cmp -s "$app_storage_config" "$app_storage_pending"; then
  rm -f -- "$app_storage_pending"
  echo "managed App storage enrollment conflicts with the installed owner" >&2; exit 1
fi
node - "$app_storage_pending" "$app_storage_config" <<'NODE'
const fs = require("node:fs")
const path = require("node:path")
const [from, to] = process.argv.slice(2)
const file = fs.openSync(from, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW)
try { fs.fsyncSync(file) } finally { fs.closeSync(file) }
fs.renameSync(from, to)
const directory = fs.openSync(path.dirname(to), fs.constants.O_RDONLY | fs.constants.O_DIRECTORY | fs.constants.O_NOFOLLOW)
try { fs.fsyncSync(directory) } finally { fs.closeSync(directory) }
NODE

)

# Publish only this installer's own links, including their first installation.
publish_managed_app_link() {
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
