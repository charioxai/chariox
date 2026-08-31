// Run the production Chariox controller. Only external Chromium/CDP responses
// are synthetic; kernel routing, relay encryption and controller stdio are real.
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [directory, pidFile] = process.argv.slice(2);
const { BrowserControllerStdioServer } = await import(pathToFileURL(join(directory, "browser-controller.mjs")));
const { BrowserCdpClient } = await import(pathToFileURL(join(directory, "browser-controller-cdp.mjs")));
writeFileSync(pidFile, String(process.pid), { flag: "wx" });
let open = true;
const chromium = {
  isOpen: () => open,
  close: async () => { open = false; },
  async send(method) {
    switch (method) {
      case "Target.getTargets": return { targetInfos: [{ type: "page", targetId: "worker-tab", url: "https://worker.test/", title: "Worker browser" }] };
      case "Target.attachToTarget": return { sessionId: "worker-cdp-session" };
      case "Page.getFrameTree": return { frameTree: { frame: { id: "worker-frame", loaderId: "worker-document" } } };
      case "Runtime.evaluate": return { result: { value: true } };
      case "Accessibility.getFullAXTree": return { nodes: [{
        nodeId: "ax-save", backendDOMNodeId: 103, ignored: false,
        role: { value: "button" }, name: { value: "Save on worker" },
        properties: [{ name: "focused", value: { value: true } }],
      }] };
      case "DOMSnapshot.captureSnapshot": return {
        strings: ["#document", "BUTTON", "", "Save on worker", "https://worker.test/"],
        documents: [{ documentURL: 4, nodes: {
          parentIndex: [-1, 0], nodeType: [9, 1], nodeName: [0, 1],
          nodeValue: [2, 3], backendNodeId: [100, 103], attributes: [[], []],
        }, layout: { nodeIndex: [1], bounds: [[10, 20, 100, 30]] } }],
      };
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
