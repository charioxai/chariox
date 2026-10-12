import assert from "node:assert/strict"
import { readFile } from "node:fs/promises"
import test from "node:test"

const read = path => readFile(new URL(path, import.meta.url), "utf8")

test("persistent home is hard-capped before archive extraction and writable layer before start", async () => {
  const source = await read("./provision-linux-docker-slice.sh")
  const prepareHome = source.slice(source.indexOf("prepare_home_volume()"), source.indexOf("machine_id_hex()"))
  const restore = source.slice(source.indexOf("restore_saved_home_volume()"), source.indexOf("prepare_home_volume()"))
  const ensureContainer = source.slice(source.indexOf("ensure_container()"), source.indexOf("recover_existing_container()"))

  assert.ok(prepareHome.indexOf("apply_home_disk_quota") < prepareHome.indexOf("restore_saved_home_volume"))
  assert.ok(prepareHome.indexOf("apply_home_disk_quota") < prepareHome.indexOf("docker volume create"))
  assert.ok(ensureContainer.indexOf("docker create") < ensureContainer.indexOf("apply_layer_disk_quota"))
  assert.ok(ensureContainer.indexOf("apply_both_disk_quotas") < ensureContainer.indexOf('docker start "$SLICE_NAME"'))
  assert.match(restore, /-v "\$SLICE_SAVED_HOME_ARCHIVE_DIR:\/restore:ro"/)
  assert.match(restore, /tar --zstd -C \/home-dst -xf "\$1"/)
  assert.match(restore, /run_with_file_stdin_timeout 120 "\$SLICE_SAVED_HOME_ARCHIVE"/)
  assert.doesNotMatch(restore, /docker cp/)
  assert.match(source, /quota_labels\+=\(--label "io\.chariox\.slice\.disk-quota=xfs-project-v1"\)/)
  assert.match(source, /docker_create_args\+=\(--label "io\.chariox\.slice\.disk-quota=xfs-project-v1"\)/)
  assert.doesNotMatch(source, /--storage-opt/)
})

test("recover reconciles both quotas before unpause or restart", async () => {
  const source = await read("./provision-linux-docker-slice.sh")
  const recover = source.slice(source.indexOf("recover_existing_container()"), source.indexOf("stop_slice_services()"))
  assert.ok(recover.indexOf("apply_both_disk_quotas") < recover.indexOf("docker unpause"))
  assert.ok(recover.indexOf("apply_both_disk_quotas") < recover.indexOf('docker start "$SLICE_NAME"'))
})

test("every brokered Docker start and unpause requires allocator admission before execution", async () => {
  const [source, admission] = await Promise.all([
    read("./managed-docker-broker.mjs"),
    read("./slice-disk-quota-admission.mjs"),
  ])
  assert.match(source, /runWithSliceDiskQuotaAdmission,[\s\S]*from "\.\/slice-disk-quota-admission\.mjs"/)
  const execute = source.slice(source.indexOf("async function execute(request)"), source.indexOf("function errorResponse"))
  const runPrepared = execute.indexOf("const runPrepared = async (admission) =>")
  const prepare = execute.indexOf("prepared = request.kind === \"docker\" ? prepareDocker(request.args)", runPrepared)
  const spawn = execute.indexOf("return spawnBounded(command, args", prepare)
  const dispatch = execute.indexOf('const isDockerStartOrUnpause = request.kind === "docker" && ["start", "unpause"].includes(request.args[0])')
  const admissionCall = execute.indexOf("? await sliceDiskQuotaCoordinator.withContainerLock(containerName", dispatch)
  const directRun = execute.indexOf(": await runPrepared()", admissionCall)

  assert.ok(runPrepared >= 0 && prepare > runPrepared && spawn > prepare)
  assert.ok(dispatch > runPrepared && admissionCall > dispatch && directRun > admissionCall)
  assert.ok(execute.indexOf("containerName,", admissionCall) < directRun)
  assert.ok(execute.indexOf("runWithSliceDiskQuotaAdmission({", admissionCall) < directRun)
  assert.ok(execute.indexOf("quotaMarkerPresent: diskQuotaMarkerPresent(containerName)", admissionCall) < directRun)
  assert.ok(execute.indexOf("run: async (quotaResult, admission)", admissionCall) < directRun)
  assert.equal(execute.indexOf("runWithSliceDiskQuotaAdmission", admissionCall), execute.lastIndexOf("runWithSliceDiskQuotaAdmission"))

  const requestShape = admission.indexOf('operation: "ensure_before_start"')
  const requestValidation = admission.indexOf("validateSliceDiskQuotaRequest(request)", requestShape)
  const quotaRequest = admission.indexOf("result = await requestQuota(request)", requestValidation)
  const allocatorCatch = admission.indexOf("} catch (error) {", quotaRequest)
  const unavailableGate = admission.indexOf('!new Set(["ENOENT", "ECONNREFUSED"]).has(error?.code)', allocatorCatch)
  const proofLookup = admission.indexOf("proof = await resolveUnboundedProof()", unavailableGate)
  const proofValidation = admission.indexOf('exactKeys(proof, ["containerId"]', proofLookup)
  const offlineResult = admission.indexOf("result = { bounded: false }", proofValidation)
  const resultValidation = admission.indexOf("if (!validateEnsureBeforeStartResult(result)", offlineResult)
  const markerFailClosed = admission.indexOf("quotaMarkerPresent", resultValidation)
  const admittedRun = admission.indexOf("return run(result,", markerFailClosed)
  assert.ok(requestShape >= 0 && requestValidation > requestShape)
  assert.ok(quotaRequest > requestValidation)
  assert.ok(allocatorCatch > quotaRequest && unavailableGate > allocatorCatch && proofLookup > unavailableGate)
  assert.ok(proofValidation > proofLookup && offlineResult > proofValidation)
  assert.ok(resultValidation > offlineResult && markerFailClosed > resultValidation && admittedRun > markerFailClosed)
  assert.match(admission.slice(resultValidation, admittedRun), /validateEnsureBeforeStartResult\(result\) && quotaMarkerPresent/)
})

test("quota reservations survive failed destroy and are released only after container and volume removal", async () => {
  const [broker, allocator, backend] = await Promise.all([
    read("./managed-docker-broker.mjs"),
    read("./slice-disk-quota-allocator.mjs"),
    read("./slice-disk-quota-xfs-backend.mjs"),
  ])
  const execute = broker.slice(broker.indexOf("async function execute(request)"), broker.indexOf("function errorResponse"))
  const destroy = execute.slice(execute.indexOf('request.kind === "provisioner" && request.action === "destroy"'))
  const removalSuccess = destroy.indexOf("request.action === \"destroy\" && result.status === 0")
  const release = destroy.indexOf('operation: "release"')
  assert.ok(removalSuccess >= 0 && removalSuccess < release)
  assert.match(allocator, /confirmContainerAndVolumeRemoved\(record\.identity\)/)
  assert.match(backend, /dockerJson\(\[kind, "inspect", name\]\)/)
  assert.ok(allocator.indexOf("backend.clearProjectQuota(id)") < allocator.indexOf("delete state.reservations[key]"))
  assert.match(backend, /Docker storage still has project-quota usage/)
})

test("managed saved-home restore mounts its verified archive directory read-only instead of copying it into an unbounded helper layer", async () => {
  const source = await read("./managed-docker-broker.mjs")
  assert.match(source, /pinnedSharedPath\(dirname\(candidate\), "managed saved home archive generation", "directory"\)/)
  assert.match(source, /entries\[0\] !== "home\.tar\.zst" \|\| entries\[1\] !== "metadata\.json"/)
  assert.match(source, /environment\.CHARIOX_SLICE_BROKER_SAVED_HOME_ARCHIVE_DIR = pinned\.directory\.path/)
})

test("fresh quota driver selection is gated by the exact XFS project-quota mount", async () => {
  const [service, allocator, backend, unit, installer, imagePrep] = await Promise.all([
    read("./managed-rootless-service.sh"),
    read("./slice-disk-quota-allocator.mjs"),
    read("./slice-disk-quota-xfs-backend.mjs"),
    read("./chariox-slice-disk-quota-allocator.service"),
    read("../../../deploy/managed-kernel/install-image.sh"),
    read("../../../deploy/managed-kernel/prepare-hetzner-image.sh"),
  ])
  assert.match(service, /\[ \"\$1\" = \"\$ROOTLESS_DATA_ROOT\" \] && \[ \"\$2\" = xfs \]/)
  assert.match(service, /pquota,\*\|\*,prjquota,\*/)
  assert.match(service, /ftype=1/)
  assert.ok(service.includes('containerd-snapshotter":false'))
  assert.ok(service.includes('storage-driver":"overlay2'))
  assert.match(backend, /info\.Driver !== \"overlay2\"/)
  assert.match(backend, /xfs_info[\s\S]*ftype=1/)
  assert.match(backend, /graph\?\.Name !== \"overlay2\" \|\| typeof graph\.Data\?\.UpperDir !== \"string\"/)
  assert.match(backend, /assertTrustedPath\(upperDir, layerRoot, dockerUid\)[\s\S]*assertTrustedPath\(workDir, layerRoot, dockerUid\)/)
  assert.match(backend, /for \(const targetPath of trustedPaths\)[\s\S]*project -s -p \$\{targetPath\}/)
  assert.match(backend, /paths: \[[\s\S]*upperDir[\s\S]*workDir/)
  assert.match(allocator, /paths: layerTarget\.paths,[\s\S]*projectId: record\.projectIds\.writableLayer/)
  assert.match(backend, /limit -p bhard=\$\{limitBytes\} \$\{projectId\}/)
  assert.match(backend, /effectiveLimitBytes: quota\.hardLimitBytes/)
  assert.match(backend, /!alreadyBound && !\["absent", "created", "exited", "dead"\]\.includes\(state\)/)
  assert.match(unit, /CapabilityBoundingSet=CAP_SYS_ADMIN/)
  assert.match(unit, /Group=chariox-docker/)
  assert.doesNotMatch(unit, /BindReadOnlyPaths=.*allocator\.sock/)
  assert.match(unit, /slice-disk-quota-service\.mjs/)
  assert.match(installer, /slice-disk-quota-xfs-backend\.mjs/)
  assert.match(imagePrep, /xfsprogs/)
})

test("shared allocator coordination helper is packaged and writable to the broker only at its state root", async () => {
  const [brokerUnit, installer] = await Promise.all([
    read("../../../deploy/managed-kernel/chariox-slice-broker.service"),
    read("../../../deploy/managed-kernel/install-image.sh"),
  ])
  assert.match(brokerUnit, /ReadWritePaths=.*\/var\/lib\/chariox-slice-disk-quota/)
  assert.match(installer, /slice-disk-quota-coordinator\.mjs/)
})

test("MP-03/MP-10/MP-11 paired disk-cap fields remain covered by union protocol 491", async () => {
  const [rust, client, snapshot] = await Promise.all([
    read("../src/local/api/types.rs"),
    read("../../../packages/kernel-client/src/kernel-types.ts"),
    read("../src/local/api/tests/protocol_shapes/slice_disk_quota.rs"),
  ])
  const runtimeVersion = Number(rust.match(/LOCAL_DAEMON_PROTOCOL_VERSION: u32 = (\d+)/)?.[1])
  const clientVersion = Number(client.match(/LOCAL_DAEMON_PROTOCOL_VERSION = (\d+)/)?.[1])
  const snapshotVersion = Number(snapshot.match(/assert_eq!\(LOCAL_DAEMON_PROTOCOL_VERSION, (\d+)\)/)?.[1])
  assert.equal(runtimeVersion, 491)
  assert.equal(clientVersion, runtimeVersion)
  assert.equal(snapshotVersion, runtimeVersion)
  assert.match(snapshot, /disk_layer_mb/)
  assert.match(snapshot, /disk_home_mb/)
})
