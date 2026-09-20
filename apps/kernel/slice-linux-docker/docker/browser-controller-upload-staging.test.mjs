import test from "node:test";
import assert from "node:assert/strict";

import {
  UploadArtifactLeaseStore,
  UploadStagingError,
  UPLOAD_ARTIFACT_KIND,
  UPLOAD_STAGING_ERROR_CODES,
} from "./browser-controller-upload-staging.mjs";

function makeBroker() {
  let nextId = 1;
  const artifacts = new Map();
  const requests = [];
  const released = [];
  return {
    separate_uid: true,
    requests,
    released,
    stage: async (request) => {
      requests.push(request);
      const artifactId = `artifact-${nextId++}`;
      const path = `/broker-owned/${artifactId}`;
      artifacts.set(path, true);
      return {
        kind: UPLOAD_ARTIFACT_KIND,
        artifact_id: artifactId,
        path,
        size: request.expectedSize,
        immutable: true,
        sealed: true,
        separate_uid: true,
        broker_owned: true,
        release: async () => {
          if (!artifacts.get(path)) return;
          artifacts.delete(path);
          released.push(path);
        },
      };
    },
    isLive(path) {
      return artifacts.has(path);
    },
  };
}

const INPUT = Object.freeze({
  tab_id: "tab-1",
  document_id: "document-1",
  backend_node_id: 7,
});

function artifactRequest(bytes) {
  return {
    bytes: Buffer.from(bytes),
    expectedSize: bytes.length,
    maxBytes: 1024,
  };
}

test("stages only bounded bytes and rejects pathname-only artifacts", async () => {
  const broker = makeBroker();
  const store = new UploadArtifactLeaseStore({ broker, maxAgeMs: 1000 });
  const bytes = Buffer.from("safe bytes");
  const artifact = await store.stage(artifactRequest(bytes));
  assert.equal(broker.requests[0].sourcePath, undefined);
  assert.deepEqual(Buffer.from(broker.requests[0].bytes), bytes);
  assert.equal(artifact.immutable, true);
  assert.equal(artifact.sealed, true);
  assert.equal(artifact.broker_owned, true);

  const pathnameBroker = {
    separate_uid: true,
    stage: async () => ({
      kind: UPLOAD_ARTIFACT_KIND,
      artifact_id: "pathname",
      path: "/tmp/mutable",
      size: 1,
      immutable: true,
      sealed: true,
      separate_uid: false,
      broker_owned: true,
      release: async () => {},
    }),
  };
  await assert.rejects(
    new UploadArtifactLeaseStore({ broker: pathnameBroker }).stage(artifactRequest(Buffer.from("x"))),
    (error) => error instanceof UploadStagingError && error.code === UPLOAD_STAGING_ERROR_CODES.INVALID_ARTIFACT,
  );
});

test("replaces one input, releases stale documents and tabs, and shuts down all leases", async () => {
  const broker = makeBroker();
  const store = new UploadArtifactLeaseStore({ broker, maxArtifacts: 4, maxBytes: 100, maxAgeMs: 1000 });
  const first = await store.stage(artifactRequest(Buffer.from("one")));
  await store.replace(INPUT, [first]);
  assert.deepEqual(store.snapshot(), { count: 1, bytes: 3 });
  assert.equal(broker.isLive(first.path), true);

  const second = await store.stage(artifactRequest(Buffer.from("two")));
  await store.assertCanReplace(INPUT, [second]);
  await store.replace(INPUT, [second]);
  assert.equal(broker.isLive(first.path), false);
  assert.equal(broker.isLive(second.path), true);

  const otherDocument = await store.stage(artifactRequest(Buffer.from("old")));
  await store.replace({ ...INPUT, document_id: "document-2", backend_node_id: 8 }, [otherDocument]);
  await store.releaseForOtherDocuments({ tab_id: INPUT.tab_id, document_id: INPUT.document_id });
  assert.equal(broker.isLive(otherDocument.path), false);
  assert.equal(broker.isLive(second.path), true);

  await store.releaseForTab(INPUT.tab_id);
  assert.deepEqual(store.snapshot(), { count: 0, bytes: 0 });
  assert.equal(broker.isLive(second.path), false);

  const final = await store.stage(artifactRequest(Buffer.from("final")));
  await store.replace(INPUT, [final]);
  await store.shutdown();
  assert.equal(broker.isLive(final.path), false);
  assert.deepEqual(store.snapshot(), { count: 0, bytes: 0 });
});

test("expires leases deterministically and enforces aggregate limits", async () => {
  let now = 0;
  let nextTimer = 1;
  const timers = new Map();
  const broker = makeBroker();
  const store = new UploadArtifactLeaseStore({
    broker,
    now: () => now,
    maxArtifacts: 1,
    maxBytes: 4,
    maxAgeMs: 10,
    setTimeout: (callback) => {
      const id = nextTimer++;
      timers.set(id, callback);
      return { id, unref() {} };
    },
    clearTimeout: (timer) => timers.delete(timer?.id),
  });
  const first = await store.stage(artifactRequest(Buffer.from("1234")));
  await store.replace(INPUT, [first]);
  const second = await store.stage(artifactRequest(Buffer.from("x")));
  await assert.rejects(
    store.assertCanReplace({ ...INPUT, backend_node_id: 8 }, [second]),
    (error) => error instanceof UploadStagingError && error.code === UPLOAD_STAGING_ERROR_CODES.LIMIT,
  );

  now = 10;
  await store.expire();
  assert.equal(broker.isLive(first.path), false);
  await store.replace({ ...INPUT, backend_node_id: 8 }, [second]);
  assert.deepEqual(store.snapshot(), { count: 1, bytes: 1 });
});
