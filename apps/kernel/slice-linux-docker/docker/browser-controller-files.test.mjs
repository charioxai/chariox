import assert from "node:assert/strict";
import test from "node:test";

import {
  configureBrowserDownloads,
  uploadBrowserFiles,
} from "./browser-controller-files.mjs";

test("downloads use one configured private directory and GUID filenames", async () => {
  const connection = new FakeFileConnection();
  const fileSystem = new FakeFileSystem({
    "/safe/downloads": { realpath: "/safe/downloads", type: "directory" },
  });

  const result = await configureBrowserDownloads({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    downloadDirectory: "/safe/downloads",
    fileSystem,
  });

  assert.deepEqual(result, {
    target_id: "target-a",
    document_id: "loader-a",
    enabled: true,
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "Browser.setDownloadBehavior"),
    {
      method: "Browser.setDownloadBehavior",
      params: {
        behavior: "allowAndName",
        downloadPath: "/safe/downloads",
        eventsEnabled: true,
      },
      sessionId: undefined,
    },
  );
  assert.deepEqual(fileSystem.created, ["/safe/downloads"]);
});

test("downloads recheck the document after directory setup before changing browser policy", async () => {
  const connection = new FakeFileConnection();
  const fileSystem = new FakeFileSystem({
    "/safe/downloads": { realpath: "/safe/downloads", type: "directory" },
  });
  const inspect = fileSystem.stat.bind(fileSystem);
  fileSystem.stat = async (path) => {
    const metadata = await inspect(path);
    connection.loaderId = "loader-b";
    return metadata;
  };
  await assert.rejects(configureBrowserDownloads({
    connection, sessionId: "session-a", targetId: "target-a", documentId: "loader-a",
    downloadDirectory: "/safe/downloads", fileSystem,
  }), (error) => error.code === "stale_document_reference");
  assert.equal(connection.calls.some((call) => call.method === "Browser.setDownloadBehavior"), false);
});

test("uploads resolve regular files inside configured roots without returning paths", async () => {
  const connection = new FakeFileConnection();
  const fileSystem = new FakeFileSystem({
    "/safe/uploads": { realpath: "/safe/uploads", type: "directory" },
    "/safe/uploads/report.txt": {
      realpath: "/safe/uploads/report.txt",
      type: "file",
      size: 12,
    },
  });

  const result = await uploadBrowserFiles({
    connection,
    sessionId: "session-a",
    targetId: "target-a",
    documentId: "loader-a",
    nodeRef: "backend:103",
    filePaths: ["/safe/uploads/report.txt"],
    uploadRoots: ["/safe/uploads"],
    fileSystem,
  });

  assert.deepEqual(result, {
    target_id: "target-a",
    document_id: "loader-a",
    file_count: 1,
    total_bytes: 12,
  });
  assert.deepEqual(
    connection.calls.find((call) => call.method === "DOM.setFileInputFiles"),
    {
      method: "DOM.setFileInputFiles",
      params: {
        objectId: "file-object",
        files: ["/safe/uploads/report.txt"],
      },
      sessionId: "session-a",
    },
  );
  assert.equal(JSON.stringify(result).includes("/safe"), false);
  assert.deepEqual(connection.calls.find((call) => call.method === "DOM.resolveNode"), {
    method: "DOM.resolveNode", params: { backendNodeId: 103 }, sessionId: "session-a",
  });
  assert.deepEqual(connection.calls.at(-1), {
    method: "Runtime.releaseObject", params: { objectId: "file-object" }, sessionId: "session-a",
  });
});

for (const failure of ["detached", "invalid", "exception", "set-files-failed"]) {
test(`upload ${failure} releases its resolved object without reporting success`, async () => {
  const connection = new FakeFileConnection();
  connection.failure = failure;
  const fileSystem = new FakeFileSystem({
    "/safe/uploads": { realpath: "/safe/uploads", type: "directory" },
    "/safe/uploads/report.txt": { realpath: "/safe/uploads/report.txt", type: "file", size: 12 },
  });
  await assert.rejects(uploadBrowserFiles({
    connection, sessionId: "session-a", targetId: "target-a", documentId: "loader-a",
    nodeRef: "backend:103", filePaths: ["/safe/uploads/report.txt"], uploadRoots: ["/safe/uploads"], fileSystem,
  }), (error) => error.code === (failure === "invalid" ? "browser_upload_invalid" : "stale_element_reference"));
  assert.equal(connection.calls.filter((call) => call.method === "DOM.setFileInputFiles").length, failure === "set-files-failed" ? 1 : 0);
  assert.deepEqual(connection.calls.at(-1), {
    method: "Runtime.releaseObject", params: { objectId: "file-object" }, sessionId: "session-a",
  });
});
}

test("file transfer rejects stale documents, unsafe roots, and oversized sets", async () => {
  const connection = new FakeFileConnection();
  const fileSystem = new FakeFileSystem({
    "/safe/uploads": { realpath: "/safe/uploads", type: "directory" },
    "/safe/uploads/link.txt": {
      realpath: "/outside/secret.txt",
      type: "file",
      size: 1,
    },
  });

  await assert.rejects(
    uploadBrowserFiles({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      filePaths: ["/safe/uploads/link.txt"],
      uploadRoots: ["/safe/uploads"],
      fileSystem,
    }),
    (error) => error.code === "browser_upload_denied",
  );

  await assert.rejects(
    uploadBrowserFiles({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      nodeRef: "backend:103",
      filePaths: Array.from({ length: 21 }, (_, index) => `/safe/uploads/${index}`),
      uploadRoots: ["/safe/uploads"],
      fileSystem,
    }),
    (error) => error.code === "browser_upload_invalid",
  );

  connection.loaderId = "loader-b";
  await assert.rejects(
    configureBrowserDownloads({
      connection,
      sessionId: "session-a",
      targetId: "target-a",
      documentId: "loader-a",
      downloadDirectory: "/safe/downloads",
      fileSystem,
    }),
    (error) => error.code === "stale_document_reference",
  );
});

class FakeFileConnection {
  constructor() {
    this.calls = [];
    this.loaderId = "loader-a";
  }

  async send(method, params = {}, sessionId) {
    this.calls.push({ method, params, sessionId });
    if (method === "Page.getFrameTree") {
      return { frameTree: { frame: { loaderId: this.loaderId } } };
    }
    if (method === "DOM.resolveNode") return { object: { objectId: "file-object" } };
    if (method === "Runtime.callFunctionOn") {
      if (this.failure === "exception") return { exceptionDetails: {} };
      return { result: { value: ["detached", "invalid"].includes(this.failure) ? this.failure : "file" } };
    }
    if (method === "DOM.setFileInputFiles" && this.failure === "set-files-failed") {
      throw Object.assign(new Error("input was destroyed"), { code: "browser_cdp_command_failed" });
    }
    if (method === "Runtime.releaseObject") return {};
    if (method === "Browser.setDownloadBehavior" || method === "DOM.setFileInputFiles") {
      return {};
    }
    throw new Error(`unexpected CDP method ${method}`);
  }
}

class FakeFileSystem {
  constructor(entries) {
    this.entries = entries;
    this.created = [];
  }

  async mkdir(path) {
    this.created.push(path);
  }

  async realpath(path) {
    const entry = this.entries[path];
    if (!entry) throw Object.assign(new Error("not found"), { code: "ENOENT" });
    return entry.realpath;
  }

  async stat(path) {
    const entry = Object.values(this.entries).find((candidate) => candidate.realpath === path);
    if (!entry) throw Object.assign(new Error("not found"), { code: "ENOENT" });
    return {
      size: entry.size ?? 0,
      isFile: () => entry.type === "file",
      isDirectory: () => entry.type === "directory",
    };
  }
}
