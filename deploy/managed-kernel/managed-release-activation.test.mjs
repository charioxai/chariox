import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const installSource = await readFile(new URL("./install-image.sh", import.meta.url), "utf8")
const upgradeSource = await readFile(new URL("./upgrade-image.sh", import.meta.url), "utf8")

function indexOf(source, text, label, from = 0) {
  const index = source.indexOf(text, from)
  assert.notEqual(index, -1, `${label} must be present`)
  return index
}

test("install refuses to replace an invalid digest-named release", () => {
  const publishedBranch = installSource.slice(
    indexOf(installSource, 'if [ -e "$published_release" ] || [ -L "$published_release" ]; then', "published release check"),
    indexOf(installSource, 'if [ ! -e "$published_release" ]; then', "new release branch"),
  )

  assert.match(publishedBranch, /verify_selected_release/)
  assert.match(publishedBranch, /refus(?:e|ing).*immutable.*release/i)
  assert.match(publishedBranch, /exit 1/)
  assert.doesNotMatch(publishedBranch, /rm -rf.*published_release/)
})

test("install names releases from the validated manifest digest", () => {
  assert.ok(installSource.includes("release_name=${expected_release_digest#sha256:}"))
  assert.ok(installSource.includes("published_release=$releases_root/$release_name"))
  assert.ok(installSource.includes('verify_selected_release "$image_root" "$expected_release_digest" "$trusted_public_key"'))
  assert.ok(installSource.includes('verify_selected_release "$published_release" "$expected_release_digest" "$trusted_public_key"'))
  assert.ok(installSource.includes('"$@" path1 "$trusted_builder_public_key"'))
  assert.ok(upgradeSource.includes('"$@" path1 "$trusted_builder_public_key"'))
})

test("install durably publishes the verified release before activating current", () => {
  const verify = indexOf(installSource, '"$pending_release" "$expected_release_digest" "$trusted_public_key"', "pending release verification")
  const syncTree = indexOf(installSource, 'sync-tree "$pending_release"', "release tree sync")
  const publish = indexOf(installSource, 'mv "$pending_release" "$published_release"', "release publication")
  const syncReleases = indexOf(installSource, 'sync-directory "$releases_root"', "release directory sync", publish)
  const activate = indexOf(installSource, 'atomic_symlink "releases/$release_name" "$install_root/usr/lib/chariox/current"', "current activation")

  assert.ok(verify < syncTree, "signature and digest verification precedes release syncing")
  assert.ok(syncTree < publish, "release contents are durable before digest-name publication")
  assert.ok(publish < syncReleases, "release-name publication is made durable")
  assert.ok(syncReleases < activate, "current is not activated before the release is durable")
})

test("install symlink replacement is atomic and durably synced", () => {
  const start = indexOf(installSource, "atomic_symlink() {", "atomic symlink helper")
  const end = indexOf(installSource, "releases_root=", "release installation")
  const helper = installSource.slice(start, end)

  assert.match(helper, /renameSync\(source, destination\)/)
  const rename = indexOf(helper, "renameSync(source, destination)", "atomic rename")
  const sync = indexOf(helper, 'sync-directory "$(dirname "$link_path")"', "parent directory sync", rename)
  assert.ok(rename < sync, "directory sync follows the atomic symlink rename")
})

test("install activation failure restores the prior current release", () => {
  const activation = installSource.slice(indexOf(installSource, 'atomic_symlink "releases/$release_name"', "current activation"))
  assert.ok(installSource.includes('atomic_symlink "$previous_current_target"'))
  assert.ok(activation.includes("if restore_previous_current; then"))
  assert.ok(activation.includes("managed release activation failed; restored previous current release"))
})

test("install rolls back if the current symlink or its directory sync fails", () => {
  assert.ok(installSource.includes("restore_previous_current() {"))
  const activation = installSource.slice(indexOf(installSource, 'if ! atomic_symlink "releases/$release_name"', "guarded current activation"))
  assert.ok(activation.includes("restore_previous_current"))
  assert.ok(activation.includes("current-link activation failed"))
})

test("upgrade durably publishes a verified release before creating its transaction", () => {
  const verify = indexOf(upgradeSource, '"$pending_release" "$expected_new_digest" "$next_trusted_public_key"', "pending release verification")
  const syncTree = indexOf(upgradeSource, 'sync-tree "$pending_release"', "pending release tree sync")
  const publish = indexOf(upgradeSource, 'mv "$pending_release" "$published_release"', "published release rename")
  const syncDirectory = indexOf(upgradeSource, 'sync-directory "$releases_root"', "release parent sync", publish)
  const transaction = indexOf(upgradeSource, 'pending_transaction=$chariox_root/.managed-kernel-upgrade.pending', "transaction preparation")

  assert.ok(verify < syncTree)
  assert.ok(syncTree < publish)
  assert.ok(publish < syncDirectory)
  assert.ok(syncDirectory < transaction)
})

test("upgrade verifies signed current, staged, and already-published releases before activation", () => {
  const currentVerify = indexOf(upgradeSource, '"$releases_root/${expected_current_digest#sha256:}" "$expected_current_digest" "$trusted_public_key"', "current release verification")
  const imageVerify = indexOf(upgradeSource, '"$image_root" "$expected_new_digest" "$next_trusted_public_key"', "incoming release verification")
  const publishedVerify = indexOf(upgradeSource, '"$published_release" "$expected_new_digest" "$next_trusted_public_key"', "published release verification")
  const stagedVerify = indexOf(upgradeSource, '"$pending_release" "$expected_new_digest" "$next_trusted_public_key"', "staged release verification")
  const activation = indexOf(upgradeSource, 'atomic_symlink "releases/$release_name" "$current_link"', "upgrade current activation")
  const transaction = indexOf(upgradeSource, 'pending_transaction=$chariox_root/.managed-kernel-upgrade.pending', "transaction preparation")
  const stop = indexOf(upgradeSource, 'if ! systemctl stop "$service_name"; then', "service stop", transaction)

  assert.ok(currentVerify < imageVerify)
  assert.ok(imageVerify < publishedVerify)
  assert.ok(imageVerify < stagedVerify)
  assert.ok(publishedVerify < activation)
  assert.ok(stagedVerify < activation)
  assert.ok(imageVerify < stop)
  assert.ok(stop < activation)
  assert.ok(upgradeSource.includes('node "$script_root/managed-kernel-upgrade-state.mjs" atomic-symlink "$1" "$2"'))
})

test("upgrade recovers interrupted phases and rolls back failed migration or health checks", () => {
  assert.ok(upgradeSource.includes("prepared|stopped|activated) rollback_transaction"))
  assert.ok(upgradeSource.includes("recover_transaction\nselect_receipt_path"))
  assert.ok(upgradeSource.includes('"$terminal_transaction/phase"'))
  assert.ok(upgradeSource.includes("if ! resume_home_migration; then"))
  assert.ok(upgradeSource.includes("managed kernel home migration failed; restored previous managed kernel release"))
  const healthFailure = upgradeSource.slice(indexOf(upgradeSource, '|| ! check_health "$target_protocol"', "target health check"))
  assert.ok(healthFailure.indexOf("rollback_transaction") > 0, "failed health checks trigger rollback")
  assert.ok(upgradeSource.includes('atomic_symlink "$previous_target" "$current_link"'))
  assert.ok(upgradeSource.includes("write_phase committed"))
  assert.ok(upgradeSource.includes("write_phase rolled_back"))
})

test("release paths stay separate from mutable managed home state", () => {
  assert.ok(installSource.includes("managed_home=$install_root/home/chariox"))
  assert.ok(installSource.includes("managed_state=$managed_home/.chariox"))
  assert.ok(installSource.includes("releases_root=$install_root/usr/lib/chariox/releases"))
  assert.ok(upgradeSource.includes("managed_home=$install_root/home/chariox"))
  assert.ok(upgradeSource.includes("managed_state=$managed_home/.chariox"))
  assert.ok(upgradeSource.includes("releases_root=$chariox_root/releases"))
  assert.doesNotMatch(`${installSource}\n${upgradeSource}`, /rm -rf -- "\$managed_(?:home|state)(?:\/|"|\s)/)
})
