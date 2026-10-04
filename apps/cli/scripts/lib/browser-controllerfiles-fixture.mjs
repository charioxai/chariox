// MP-08/MP-10/MP-11: first-party byte receipts, never a benchmark evaluator.
import { createHash } from "node:crypto";
import { createServer } from "node:http";
import { parseFixtureMail } from "./browser-computer-fixture-mail.mjs";

const MAX_BODY = 1024 * 1024;
const TEXT = Buffer.from("Chariox artifact\nGrüße 世界\n");
const BINARY = Buffer.from([0, 255, 13, 10, 65, 128]);
const PDF = samplePdf();

export function controllerfilesSamples() {
  return [
    { name: "report.txt", type: "text/plain", bytes: Buffer.from(TEXT) },
    { name: "binary.bin", type: "application/octet-stream", bytes: Buffer.from(BINARY) },
    { name: "document.pdf", type: "application/pdf", bytes: Buffer.from(PDF) },
  ];
}

export function assertControllerfilesReceipt(actual, expected) {
  if (!Array.isArray(actual) || actual.length !== expected.length) throw new Error("attachment count mismatch");
  for (let i = 0; i < expected.length; i++) {
    const file = expected[i];
    const receipt = actual[i];
    const digest = createHash("sha256").update(file.bytes).digest("hex");
    if (receipt?.name !== file.name || receipt.contentType !== file.type
      || receipt.sizeBytes !== file.bytes.length || receipt.sha256 !== digest) {
      throw new Error(`attachment provenance mismatch at index ${i}`);
    }
  }
}

export async function startControllerfilesFixture() {
  const receipts = [];
  const network = [];
  const server = createServer(async (request, response) => {
    const url = new URL(request.url, "http://127.0.0.1");
    try {
      if (request.method === "GET" && url.pathname === "/") return send(response, 200, page, "text/html; charset=utf-8");
      if (request.method === "GET" && url.pathname === "/receipts") return json(response, 200, { receipts });
      if (request.method === "GET" && url.pathname === "/downloads/slow.bin") {
        response.writeHead(200, { "content-type": "application/octet-stream", "content-length": 1024 * 1024,
          "content-disposition": 'attachment; filename="slow.bin"' });
        let sent = 0;
        const timer = setInterval(() => {
          response.write(Buffer.alloc(8192, 37));
          sent += 8192;
          if (sent === 1024 * 1024) { clearInterval(timer); response.end(); }
        }, 25);
        response.once("close", () => clearInterval(timer));
        return;
      }
      if (request.method === "GET" && url.pathname.startsWith("/downloads/")) {
        const name = url.pathname.slice("/downloads/".length);
        const sample = controllerfilesSamples().find(item => item.name === name);
        if (!sample) return send(response, 404, "missing fixture");
        response.setHeader("content-disposition", `attachment; filename="${sample.name}"`);
        return send(response, 200, sample.bytes, sample.type);
      }
      if (request.method === "POST" && url.pathname === "/upload") {
        if (Number(request.headers["content-length"]) > MAX_BODY) {
          request.resume();
          return json(response, 413, { error: "fixture body limit" });
        }
        const body = await readBody(request);
        let parsed;
        try {
          if (!/^multipart\/form-data;/i.test(request.headers["content-type"] ?? "")) throw new Error("multipart required");
          parsed = await parseFixtureMail(body, request.headers["content-type"]);
          if (!parsed.attachments?.length) throw new Error("files required");
        } catch { return json(response, 400, { error: "invalid upload" }); }
        if (receipts.length === 32) return json(response, 429, { error: "fixture receipt limit" });
        receipts.push(parsed.attachments);
        return json(response, 200, { attachments: parsed.attachments });
      }
      if (request.method === "GET" && url.pathname === "/network-proof") {
        // Retain presence only. Passive captures must derive headers from CDP,
        // not this verifier or synthetic HAR fields.
        network.push({ method: request.method, path: url.pathname,
          cookiePresent: Boolean(request.headers.cookie), authPresent: Boolean(request.headers.authorization) });
        response.setHeader("set-cookie", "fixture_private=synthetic; HttpOnly; SameSite=Strict; Path=/");
        return json(response, 200, { observed: true });
      }
      send(response, 404, "missing fixture");
    } catch (error) {
      json(response, error.code === "BODY_LIMIT" ? 413 : 500, { error: "fixture request failed" });
    }
  });
  await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
  return { origin: `http://127.0.0.1:${server.address().port}`, receipts, network,
    close: () => new Promise((resolve, reject) => {
      server.close(error => error ? reject(error) : resolve());
      server.closeAllConnections();
    }) };
}

async function readBody(request) {
  let size = 0;
  const chunks = [];
  for await (const chunk of request) {
    size += chunk.length;
    if (size > MAX_BODY) throw Object.assign(new Error("bounded fixture upload"), { code: "BODY_LIMIT" });
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

function send(response, status, body, type = "text/plain") {
  response.writeHead(status, { "content-type": type, "cache-control": "no-store" });
  response.end(body);
}
function json(response, status, value) { return send(response, status, JSON.stringify(value), "application/json"); }

// A complete, deterministic one-page PDF with valid byte offsets. It is a
// file fixture, never a stand-in for product extraction or a generated score.
function samplePdf() {
  const stream = "BT /F1 18 Tf 40 700 Td (Chariox bounded artifact) Tj ET\n";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>",
    `<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}endstream`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  let document = "%PDF-1.4\n";
  const offsets = [];
  for (const [index, object] of objects.entries()) {
    offsets.push(Buffer.byteLength(document));
    document += `${index + 1} 0 obj\n${object}\nendobj\n`;
  }
  const xref = Buffer.byteLength(document);
  document += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`;
  document += offsets.map(offset => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  document += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(document);
}

const page = `<!doctype html><html><head><title>Chariox file and image fixture</title></head><body>
<h1>Shared browser artifacts</h1>
<form id="intake"><input id="attachment" type="file" name="attachment" hidden multiple>
<button id="choose" type="button">Choose documents</button>
<button id="cancel" type="button">Cancel selection</button>
<button id="send" type="submit">Send documents</button></form>
<output id="result">No documents selected</output>
<a href="/downloads/report.txt">Download report</a><a href="/downloads/binary.bin">Download binary</a>
<a href="/downloads/document.pdf">Download PDF</a>
<a href="/downloads/slow.bin">Download slowly</a>
<canvas id="visual" width="320" height="180" aria-label="Visual capture fixture"></canvas>
<script>
const input = document.querySelector('#attachment');
document.querySelector('#choose').onclick = () => input.click();
document.querySelector('#cancel').onclick = () => { input.value = ''; document.querySelector('#result').textContent = 'Selection cancelled'; };
input.onchange = () => { document.querySelector('#result').textContent = input.files.length + ' documents selected'; };
document.querySelector('#intake').onsubmit = async event => {
 event.preventDefault(); const response = await fetch('/upload', { method: 'POST', body: new FormData(event.target) });
 document.querySelector('#result').textContent = response.ok ? 'Documents received' : 'Upload rejected';
};
const ctx = document.querySelector('#visual').getContext('2d');
ctx.fillStyle = '#1347b2'; ctx.fillRect(0,0,320,180);
ctx.fillStyle = '#f7d22c'; ctx.fillRect(32,24,80,48);
ctx.fillStyle = '#d83539'; ctx.beginPath(); ctx.arc(240,120,28,0,Math.PI*2); ctx.fill();
</script></body></html>`;
