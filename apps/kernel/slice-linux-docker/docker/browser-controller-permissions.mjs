const PERMISSION_DESCRIPTORS = new Map([
  ["camera", { name: "videoCapture" }],
  ["clipboard-read-write", { name: "clipboardReadWrite" }],
  ["clipboard-sanitized-write", { name: "clipboardSanitizedWrite" }],
  ["display-capture", { name: "displayCapture" }],
  ["geolocation", { name: "geolocation" }],
  ["local-fonts", { name: "localFonts" }],
  ["microphone", { name: "audioCapture" }],
  ["midi", { name: "midi" }],
  ["midi-sysex", { name: "midiSysex" }],
  ["notifications", { name: "notifications" }],
]);
const PERMISSION_SETTINGS = new Set(["granted", "denied", "prompt"]);

export class BrowserPermissionError extends Error {
  constructor(code, message) {
    super(message);
    this.name = "BrowserPermissionError";
    this.code = code;
  }
}

export async function setBrowserPermission({
  connection,
  sessionId,
  targetId,
  documentId,
  targetUrl,
  permission,
  setting,
}) {
  const descriptor = PERMISSION_DESCRIPTORS.get(permission);
  if (!descriptor || !PERMISSION_SETTINGS.has(setting)) {
    throw new BrowserPermissionError(
      "browser_permission_invalid",
      "browser permission name or setting is not supported",
    );
  }
  const frameTree = await connection.send("Page.getFrameTree", {}, sessionId);
  if (frameTree?.frameTree?.frame?.loaderId !== documentId) {
    throw new BrowserPermissionError(
      "stale_document_reference",
      `browser target ${JSON.stringify(targetId)} moved away from the requested document`,
    );
  }
  let origin;
  try {
    const url = new URL(targetUrl);
    if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error("unsafe scheme");
    origin = url.origin;
  } catch {
    throw new BrowserPermissionError(
      "browser_permission_origin_denied",
      "browser permissions require the current HTTP or HTTPS origin",
    );
  }
  await connection.send("Browser.setPermission", {
    permission: descriptor,
    setting,
    origin,
  });
  return {
    target_id: targetId,
    document_id: documentId,
    permission,
    setting,
  };
}
