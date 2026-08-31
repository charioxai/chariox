// Run the production Chariox controller. Only external Chromium/CDP responses
// are synthetic; kernel routing, relay encryption and controller stdio are real.
import { existsSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const [directory, pidFile] = process.argv.slice(2);
const { BrowserControllerStdioServer } = await import(pathToFileURL(join(directory, "browser-controller.mjs")));
const { BrowserCdpClient } = await import(pathToFileURL(join(directory, "browser-controller-cdp.mjs")));
writeFileSync(pidFile, String(process.pid), { flag: "wx" });
let open = true;
let saved = false;
let pressed = false;
let note = "";
let submitted = null;
let focused = "worker-save";
const chromium = {
  isOpen: () => open,
  close: async () => { open = false; },
  async send(method, params = {}) {
    switch (method) {
      case "Target.getTargets": return { targetInfos: [{ type: "page", targetId: "worker-tab", url: "https://worker.test/", title: "Worker browser" }] };
      case "Target.attachToTarget": return { sessionId: "worker-cdp-session" };
      case "Page.getFrameTree": return { frameTree: { frame: { id: "worker-frame", loaderId: "worker-document" } } };
      case "Runtime.evaluate": return { result: { value: true } };
      case "Accessibility.getFullAXTree": return { nodes: [{
        nodeId: "ax-save", backendDOMNodeId: 103, ignored: false,
        role: { value: "button" }, name: { value: submitted === null ? (saved ? "Saved on worker" : "Save on worker") : `Submitted: ${submitted}` },
        properties: [{ name: "focused", value: { value: focused === "worker-save" } }],
      }, {
        nodeId: "ax-note", backendDOMNodeId: 104, ignored: false,
        role: { value: "textbox" }, name: { value: "Worker note" }, value: { value: note },
        properties: [{ name: "focused", value: { value: focused === "worker-note" } }],
      }] };
      case "DOMSnapshot.captureSnapshot": return {
        strings: ["#document", "BUTTON", "", "Save on worker", "https://worker.test/", "INPUT", "type", "text"],
        documents: [{ documentURL: 4, nodes: {
          parentIndex: [-1, 0, 0], nodeType: [9, 1, 1], nodeName: [0, 1, 5],
          nodeValue: [2, 3, 2], backendNodeId: [100, 103, 104], attributes: [[], [], [6, 7]],
        }, layout: { nodeIndex: [1, 2], bounds: [[10, 20, 100, 30], [10, 60, 100, 30]] } }],
      };
      case "DOM.resolveNode": {
        if (![103, 104].includes(params.backendNodeId)) throw new Error("wrong worker node");
        return { object: { objectId: params.backendNodeId === 103 ? "worker-save" : "worker-note" } };
      }
      case "Runtime.callFunctionOn": {
        if (!["worker-save", "worker-note"].includes(params.objectId)) throw new Error("wrong worker object");
        // External page fault injection: keep the button disabled until the
        // test releases it. No Chariox state or controller behavior is mocked.
        if (params.objectId === "worker-save" && existsSync(join(dirname(pidFile), "hold-click"))) {
          return { result: { value: { state: "disabled" } } };
        }
        if (params.functionDeclaration.includes("requestSubmit")) {
          submitted = note;
          return { result: { value: { ok: true } } };
        }
        if (params.functionDeclaration.includes("this.focus()")) {
          focused = params.objectId;
          if (focused === "worker-note" && params.arguments?.[0]?.value) note = "";
          return { result: { value: { ok: true } } };
        }
        return { result: { value: { state: "ready", x: 60, y: params.objectId === "worker-note" ? 75 : 35, width: 100, height: 30, editable: params.objectId === "worker-note" } } };
      }
      case "Input.insertText": {
        if (focused !== "worker-note") throw new Error("worker input is not focused");
        note += params.text;
        return {};
      }
      case "Runtime.releaseObject": return {};
      case "Input.dispatchMouseEvent": {
        if (params.x !== 60 || params.y !== 35) throw new Error("wrong click coordinates");
        if (params.type === "mousePressed") pressed = true;
        if (params.type === "mouseReleased" && pressed) { saved = true; submitted = null; pressed = false; }
        return {};
      }
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
  browser: new BrowserCdpClient({ connectionFactory: async () => chromium }),
}).run();
