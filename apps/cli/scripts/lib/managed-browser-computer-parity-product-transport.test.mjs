import assert from "node:assert/strict";
import { createECDH } from "node:crypto";
import { once } from "node:events";
import test from "node:test";

const moduleUrl = new URL("./managed-browser-computer-parity-product-transport.mjs", import.meta.url);
const kernelClientDistUrl = new URL("../../../../packages/kernel-client/dist/ipc.js", import.meta.url);
const kernelRequestsDistUrl = new URL("../../../../packages/kernel-client/dist/ipc-requests.js", import.meta.url);
const relayCryptoDistUrl = new URL("../../../../packages/kernel-client/dist/relay-crypto.js", import.meta.url);

const importProductTransport = () => import(moduleUrl.href);

test("factory fails closed when the managed parity endpoint is not configured", async () => {
  const imported = await importProductTransport();
  const previousEndpoint = process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL;
  delete process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL;
  try {
    await assert.rejects(
      () => imported.createManagedBrowserComputerParityTransport({ evidenceRoot: "/tmp/managed-parity-evidence" }),
      /CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL/,
    );
  } finally {
    if (previousEndpoint === undefined) delete process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL;
    else process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL = previousEndpoint;
  }
});

test("factory requires the standard local operator authentication configuration", async () => {
  const imported = await importProductTransport();
  const previous = {
    endpoint: process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL,
    ref: process.env.CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF,
    machine: process.env.CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF,
    client: process.env.CHARIOX_MANAGED_PARITY_CLIENT_ID,
    session: process.env.CHARIOX_MANAGED_PARITY_SESSION_ID,
    token: process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN,
    tokenFile: process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE,
  };
  process.env.CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL = "ws://127.0.0.1:1";
  process.env.CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF = "kernel-1";
  process.env.CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF = "machine-1";
  process.env.CHARIOX_MANAGED_PARITY_CLIENT_ID = "managed-parity-test";
  process.env.CHARIOX_MANAGED_PARITY_SESSION_ID = "room-1";
  delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN;
  delete process.env.CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE;
  try {
    await assert.rejects(
      () => imported.createManagedBrowserComputerParityTransport({ evidenceRoot: "/tmp/managed-parity-evidence" }),
      /CHARIOX_KERNEL_LOCAL_AUTH_TOKEN(?:_FILE)?/,
    );
  } finally {
    for (const [key, value] of Object.entries({
      CHARIOX_MANAGED_PARITY_HOME_KERNEL_URL: previous.endpoint,
      CHARIOX_MANAGED_PARITY_TARGET_KERNEL_REF: previous.ref,
      CHARIOX_MANAGED_PARITY_TARGET_MACHINE_REF: previous.machine,
      CHARIOX_MANAGED_PARITY_CLIENT_ID: previous.client,
      CHARIOX_MANAGED_PARITY_SESSION_ID: previous.session,
      CHARIOX_KERNEL_LOCAL_AUTH_TOKEN: previous.token,
      CHARIOX_KERNEL_LOCAL_AUTH_TOKEN_FILE: previous.tokenFile,
    })) {
      if (value === undefined) delete process.env[key];
      else process.env[key] = value;
    }
  }
});

test("selkies.attach does not turn metadata into attachment success or read a worker Room", async () => {
  const imported = await importProductTransport();
  let sends = 0;
  const transport = imported.createManagedBrowserComputerParityTransportFromPublicClient({
    client: {
      async send() {
        sends += 1;
        throw new Error("unexpected metadata request");
      },
    },
    requestApi: {
      getSliceDisplayEndpointRequest(sliceId, options) {
        return { GetSliceDisplayEndpoint: { slice_ref: sliceId, ...options } };
      },
    },
  });

  await assert.rejects(
    () => transport.run("selkies.attach", {
      binding: {
        kernelId: "kernel-1",
        machineId: "machine-1",
        roomId: "room-1",
        environmentId: "environment-1",
      },
      client: "web",
      displayBackend: "selkies",
    }),
    /sliceId, attachmentId, and viewerPublicKey.*public display authorization/,
  );
  assert.equal(sends, 0, "missing display binding must not fall back to worker Room metadata");
});

test("real LocalIpcClient authorizes a Selkies endpoint, then fails closed without a public stream connection API", async () => {
  let LocalIpcClient;
  let getSliceDisplayEndpointRequest;
  let decryptRelayPayload;
  let encryptRelayPayload;
  let WebSocketServer;
  try {
    ({ LocalIpcClient } = await import(kernelClientDistUrl.href));
    ({ getSliceDisplayEndpointRequest } = await import(kernelRequestsDistUrl.href));
    ({ decryptRelayPayload, encryptRelayPayload } = await import(relayCryptoDistUrl.href));
    ({ WebSocketServer } = await import("ws"));
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(
      `real Selkies public-client regression requires the built kernel-client dist and existing ws dependency; ${detail}`,
      { cause: error },
    );
  }

  const server = new WebSocketServer({ port: 0 });
  await once(server, "listening");
  const address = server.address();
  assert.ok(address && typeof address === "object");
  const endpoint = `ws://127.0.0.1:${address.port}`;
  const receivedFrames = [];
  const receivedRequests = [];
  let serverError;
  server.on("connection", (socket) => {
    socket.on("message", (raw) => {
      try {
        const frame = JSON.parse(raw.toString());
        receivedFrames.push(frame);
        if (frame.kind === "client_connect") {
          assert.equal(frame.auth_token, "operator-test-token");
          assert.deepEqual(frame.target, { daemon_id: "daemon-1" });
          const daemon = createECDH("prime256v1");
          const daemonPublicKey = daemon.generateKeys().toString("base64");
          socket.daemon = daemon;
          socket.send(JSON.stringify({
            kind: "client_connected",
            target: frame.target,
            daemon_public_key: daemonPublicKey,
          }));
          return;
        }
        if (frame.kind !== "client_request") return;
        assert.ok(socket.daemon);
        const envelope = JSON.parse(decryptRelayPayload(socket.daemon.getPrivateKey(), frame.encrypted_request));
        receivedRequests.push(envelope);
        assert.deepEqual(envelope.request, getSliceDisplayEndpointRequest("slice-1", {
          sessionId: "room-1",
          attachmentId: "attachment-1",
          viewerPublicKey: "viewer-public-key",
        }));
        const response = {
          SliceDisplayEndpoint: {
            endpoint: {
              slice_id: "slice-1",
              kind: "selkies",
              url: "wss://display.example/stream-1",
              access: "tunnel",
              stream_protocol: "selkies-v1",
              stream_id: "stream-1",
              peer_public_key: "peer-key-1",
            },
          },
        };
        const encryptedResponse = encryptRelayPayload(
          envelope.sender_public_key,
          Buffer.from(JSON.stringify(response), "utf8"),
        ).payload;
        socket.send(JSON.stringify({
          kind: "client_response",
          request_id: frame.request_id,
          encrypted_response: encryptedResponse,
        }));
      } catch (error) {
        serverError = error;
        socket.close();
      }
    });
  });

  const client = new LocalIpcClient(endpoint, {
    relayAuthToken: "operator-test-token",
    targetDaemonId: "daemon-1",
  });
  const imported = await importProductTransport();
  const transport = imported.createManagedBrowserComputerParityTransportFromPublicClient({
    client,
    requestApi: { getSliceDisplayEndpointRequest },
  });
  try {
    await assert.rejects(
      () => transport.run("selkies.attach", {
        binding: {
          kernelId: "kernel-1",
          machineId: "machine-1",
          roomId: "room-1",
          environmentId: "environment-1",
        },
        client: "web",
        displayBackend: "selkies",
        sliceId: "slice-1",
        attachmentId: "attachment-1",
        viewerPublicKey: "viewer-public-key",
      }),
      /no public Selkies display-stream connection API/,
    );
    assert.ifError(serverError);
    assert.deepEqual(receivedFrames.map((frame) => frame.kind), ["client_connect", "client_request"]);
    assert.equal(receivedRequests.length, 1, "one authorized display metadata request is allowed");
    assert.equal(receivedRequests[0].request.GetSliceDisplayEndpoint.slice_ref, "slice-1");
  } finally {
    client.close();
    await new Promise((resolve) => server.close(resolve));
  }
});
