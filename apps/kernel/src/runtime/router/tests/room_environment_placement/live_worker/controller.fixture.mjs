// Run the production Chariox controller. Only external Chromium/CDP responses
// are synthetic; kernel routing, relay encryption and controller stdio are real.
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const [directory, pidFile] = process.argv.slice(2);
const { BrowserControllerStdioServer } = await import(pathToFileURL(join(directory, "browser-controller.mjs")));
const { BrowserCdpClient } = await import(pathToFileURL(join(directory, "browser-controller-cdp.mjs")));
// The production supervisor may fence and restart the controller while the
// external browser remains alive. Keep the current generation convenient for
// assertions and retain every PID so cleanup can prove that none leaked.
writeFileSync(pidFile, String(process.pid));
appendFileSync(`${pidFile}s`, `${process.pid}\n`);
const stateFile = join(dirname(pidFile), "chromium-state.json");
let state = existsSync(stateFile)
  ? JSON.parse(readFileSync(stateFile, "utf8"))
  : { open: true, saved: false, clickCount: 0, pressed: false, note: "", submitted: null, focused: "worker-save" };
state.clickCount ??= 0;
const persist = () => writeFileSync(stateFile, JSON.stringify(state));
persist();
const chromium = {
  isOpen: () => state.open,
  close: async () => { state.open = false; persist(); },
  async send(method, params = {}, sessionId) {
    switch (method) {
      case "Target.getTargets": return { targetInfos: [
        { type: "page", targetId: "worker-tab", url: "https://worker.test/", title: "Worker browser" },
        ...(state.popup ? [{ type: "page", targetId: "worker-popup", url: "https://popup.worker.test/", title: "Worker popup" }] : []),
      ] };
      case "Target.attachToTarget": return { sessionId: params.targetId === "worker-popup" ? "worker-popup-session" : "worker-cdp-session" };
      case "Page.getFrameTree": return { frameTree: { frame: {
        id: sessionId === "worker-popup-session" ? "worker-popup-frame" : "worker-frame",
        loaderId: sessionId === "worker-popup-session" ? "worker-popup-document" : "worker-document",
      } } };
      case "Runtime.evaluate": return { result: { value: true } };
      case "Accessibility.getFullAXTree": return { nodes: [{
        nodeId: "ax-save", backendDOMNodeId: 103, ignored: false,
        role: { value: "button" }, name: { value: state.submitted === null ? (state.saved ? "Saved on worker" : "Save on worker") : `Submitted: ${state.submitted}` },
        properties: [{ name: "focused", value: { value: state.focused === "worker-save" } }],
      }, {
        nodeId: "ax-note", backendDOMNodeId: 104, ignored: false,
        role: { value: "textbox" }, name: { value: "Worker note" }, value: { value: state.note },
        properties: [{ name: "focused", value: { value: state.focused === "worker-note" } }],
      }] };
      case "DOMSnapshot.captureSnapshot": return {
        strings: ["#document", "BUTTON", "", "Save on worker", "https://worker.test/", "INPUT", "type", "file", "IFRAME", "DIV", "#document-fragment", "open", "https://frame.worker.test/"],
        documents: [{ documentURL: 4, nodes: {
          parentIndex: [-1, 0, 0, 0, 0, 4, 5], nodeType: [9, 1, 1, 1, 1, 11, 1], nodeName: [0, 1, 5, 8, 9, 10, 1],
          nodeValue: [2, 3, 2, 2, 2, 2, 2], backendNodeId: [100, 103, 104, 105, 106, 107, 108], attributes: [[], [], [6, 7], [], [], [], []],
          contentDocumentIndex: { index: [3], value: [1] },
          shadowRootType: { index: [5], value: [11] },
        }, layout: { nodeIndex: [1, 2, 3, 4, 5, 6], bounds: [[10, 20, 100, 30], [10, 60, 100, 30], [10, 100, 200, 80], [230, 100, 200, 80], [230, 100, 200, 80], [240, 110, 100, 30]] } }, {
          documentURL: 12, nodes: {
            parentIndex: [-1, 0], nodeType: [9, 1], nodeName: [0, 1],
            nodeValue: [2, 2], backendNodeId: [200, 201], attributes: [[], []],
          }, layout: { nodeIndex: [1], bounds: [[20, 110, 100, 30]] },
        }],
      };
      case "DOM.resolveNode": {
        if (![103, 104, 108, 201].includes(params.backendNodeId)) throw new Error("wrong worker node");
        const objectId = new Map([
          [103, "worker-save"], [104, "worker-note"],
          [108, "worker-shadow"], [201, "worker-frame"],
        ]).get(params.backendNodeId);
        return { object: { objectId } };
      }
      case "Runtime.callFunctionOn": {
        if (!["worker-save", "worker-note", "worker-shadow", "worker-frame"].includes(params.objectId)) throw new Error("wrong worker object");
        // External page fault injection: keep the button disabled until the
        // test releases it. No Chariox state or controller behavior is mocked.
        if ((params.objectId === "worker-save" && existsSync(join(dirname(pidFile), "hold-click"))) ||
            (params.objectId === "worker-note" && existsSync(join(dirname(pidFile), "hold-fill")))) {
          return { result: { value: { state: "disabled" } } };
        }
        if (params.functionDeclaration.includes("requestSubmit")) {
          state.submitted = state.note;
          persist();
          return { result: { value: { ok: true } } };
        }
        if (params.functionDeclaration.includes("this.focus()")) {
          state.focused = params.objectId;
          if (state.focused === "worker-note" && params.arguments?.[0]?.value) state.note = "";
          persist();
          return { result: { value: { ok: true } } };
        }
        const y = new Map([
          ["worker-save", 35], ["worker-note", 75],
          ["worker-frame", 125], ["worker-shadow", 165],
        ]).get(params.objectId);
        return { result: { value: { state: "ready", x: 60, y, width: 100, height: 30, editable: params.objectId === "worker-note" } } };
      }
      case "Input.insertText": {
        if (state.focused !== "worker-note") throw new Error("worker input is not focused");
        state.note += params.text;
        persist();
        return {};
      }
      case "Runtime.releaseObject":
        if (existsSync(join(dirname(pidFile), "hold-release"))) {
          writeFileSync(join(dirname(pidFile), "release-pending"), "external browser cleanup pending");
        }
        while (existsSync(join(dirname(pidFile), "hold-release"))) {
          await new Promise(resolve => setTimeout(resolve, 10));
        }
        return {};
      case "Input.dispatchMouseEvent": {
        if (params.x !== 60 || ![35, 125, 165].includes(params.y)) throw new Error("wrong click coordinates");
        if (params.type === "mousePressed") state.pressed = true;
        if (params.type === "mouseReleased" && state.pressed) {
          if (params.y === 35) state.saved = true;
          if (params.y === 125) state.frameClicked = true;
          if (params.y === 165) {
            state.shadowClicked = true;
            state.popup = true;
          }
          state.clickCount += 1;
          state.submitted = null;
          state.pressed = false;
        }
        persist();
        return {};
      }
      case "Page.handleJavaScriptDialog": state.dialog = params; persist(); return {};
      case "Browser.setDownloadBehavior": state.downloads = params; persist(); return {};
      case "DOM.setFileInputFiles": state.upload = { backendNodeId: params.backendNodeId, fileCount: params.files.length }; persist(); return {};
      case "Browser.setPermission": state.permission = params; persist(); return {};
      case "Target.setDiscoverTargets":
      case "Page.enable":
      case "Page.setLifecycleEventsEnabled":
      case "Runtime.enable":
      case "Network.enable":
      case "Inspector.enable":
      case "Emulation.setDeviceMetricsOverride": return {};
      default: throw new Error(`Unexpected CDP command: ${method}`);
    }
  },
};
await new BrowserControllerStdioServer({
  browser: new BrowserCdpClient({
    connectionFactory: async () => chromium,
    downloadDirectory: join(dirname(pidFile), "downloads"),
    uploadRoots: [dirname(pidFile)],
  }),
}).run();
