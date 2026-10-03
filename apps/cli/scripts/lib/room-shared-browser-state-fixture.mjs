import assert from "node:assert/strict"

export const roomSharedBrowserStateFields = Object.freeze([
  "cookie", "auth", "localStorage", "indexedDB", "cacheStorage", "serviceWorker",
])

export function createRoomSharedBrowserStateFixture({ generation }) {
  assert.ok(typeof generation === "string" && /^[a-zA-Z0-9-]{1,96}$/.test(generation),
    "Browser state fixture requires a bounded nonsecret generation marker")
  const cookieName = `chariox_room_${generation}`
  const cookieValue = `${generation}-authenticated-fixture`
  const cacheName = `chariox-room-state-${generation}`
  const cacheKey = `/room-state-cache/${generation}`
  const workerCacheName = `chariox-room-state-worker-${generation}`
  const workerCacheKey = `/room-state-worker/${generation}`
  let verifyOnly = false

  function handleRequest(request, response) {
    const url = new URL(request.url, "http://room-fixture.invalid")
    if (url.pathname === "/room-state-service-worker.js") {
      if (request.method !== "GET" || url.searchParams.get("generation") !== generation) {
        response.writeHead(404).end("not found")
        return true
      }
      response.writeHead(200, {
        "content-type": "application/javascript; charset=utf-8",
        "cache-control": "no-store",
        "service-worker-allowed": "/",
      })
      response.end(serviceWorkerSource({ generation, workerCacheName, workerCacheKey, seedAllowed: !verifyOnly }))
      return true
    }
    if (url.pathname === "/room-state-auth") {
      if (request.method !== "GET" || url.searchParams.get("generation") !== generation) {
        response.writeHead(404).end("not found")
        return true
      }
      const hasCookie = requestCookie(request.headers.cookie, cookieName) === cookieValue
      if (!hasCookie && !verifyOnly) {
        response.writeHead(204, {
          "set-cookie": `${cookieName}=${cookieValue}; Path=/; HttpOnly; SameSite=Lax`,
          "cache-control": "no-store",
        }).end()
      } else if (hasCookie) {
        response.writeHead(200, { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" })
          .end("ROOM_BROWSER_AUTH=PASS")
      } else {
        response.writeHead(401, { "content-type": "text/plain; charset=utf-8", "cache-control": "no-store" })
          .end("ROOM_BROWSER_AUTH=FAIL")
      }
      return true
    }
    if (url.pathname === "/room-state-seeded") {
      if (request.method !== "POST" || url.searchParams.get("generation") !== generation
        || requestCookie(request.headers.cookie, cookieName) !== cookieValue) {
        response.writeHead(403).end("state fixture seed was not authenticated")
        return true
      }
      verifyOnly = true
      response.writeHead(204, { "cache-control": "no-store" }).end()
      return true
    }
    return false
  }

  function pageScript() {
    const values = {
      generation,
      mode: verifyOnly ? "verify" : "seed",
      cookieUrl: `/room-state-auth?generation=${encodeURIComponent(generation)}`,
      seedCompleteUrl: `/room-state-seeded?generation=${encodeURIComponent(generation)}`,
      serviceWorkerUrl: `/room-state-service-worker.js?generation=${encodeURIComponent(generation)}`,
      localKey: `chariox.room.${generation}.local`,
      localValue: `${generation}-localStorage`,
      database: `chariox-room-${generation}`,
      databaseKey: "generation",
      databaseValue: `${generation}-indexedDB`,
      store: "state",
      cacheName,
      cacheKey,
      cacheValue: `${generation}-cacheStorage`,
    }
    return browserStatePageScript(values)
  }

  return {
    generation,
    handleRequest,
    pageScript,
    get mode() { return verifyOnly ? "verify" : "seed" },
  }
}

export function assertRoomSharedBrowserStateText(text) {
  assert.equal(typeof text, "string", "agent Browser state evidence must be text")
  const matches = [...text.matchAll(/ROOM_BROWSER_STATE\s+([^\r\n]+)/g)]
  assert.equal(matches.length, 1, "agent Browser text must contain exactly one state report")
  const fields = new Map()
  for (const field of matches[0][1].trim().split(/\s+/)) {
    const match = /^(cookie|auth|localStorage|indexedDB|cacheStorage|serviceWorker)=(PASS|FAIL)$/.exec(field)
    assert.ok(match, "agent Browser state report contains an unknown or malformed marker")
    assert.ok(!fields.has(match[1]), `agent Browser state report duplicated the ${match[1]} marker`)
    fields.set(match[1], match[2])
  }
  assert.equal(fields.size, roomSharedBrowserStateFields.length,
    "agent Browser state report omitted or duplicated a marker")
  for (const field of roomSharedBrowserStateFields) {
    assert.equal(fields.get(field), "PASS", `restored Browser ${field} marker is missing or changed`)
  }
  return Object.fromEntries(fields)
}

function requestCookie(header, name) {
  if (typeof header !== "string") return undefined
  for (const part of header.split(";")) {
    const separator = part.indexOf("=")
    if (separator < 0 || part.slice(0, separator).trim() !== name) continue
    return part.slice(separator + 1).trim()
  }
  return undefined
}

function serviceWorkerSource({ generation, workerCacheName, workerCacheKey, seedAllowed }) {
  const marker = JSON.stringify(generation)
  const cache = JSON.stringify(workerCacheName)
  const key = JSON.stringify(workerCacheKey)
  const seed = seedAllowed
    ? `const storage=await caches.open(cacheName);await storage.put(cacheKey,new Response(generation));`
    : ""
  return `const generation=${marker};const cacheName=${cache};const cacheKey=${key};
self.addEventListener("install",event=>{event.waitUntil((async()=>{${seed}await self.skipWaiting()})())});
self.addEventListener("activate",event=>event.waitUntil(self.clients.claim()));
self.addEventListener("message",event=>{if(event.data?.type!=="chariox-room-state-check"||!event.ports?.[0])return;event.waitUntil((async()=>{const names=await caches.keys();if(!names.includes(cacheName)){event.ports[0].postMessage({ok:false});return}const response=await (await caches.open(cacheName)).match(cacheKey);event.ports[0].postMessage({ok:(await response?.text())===generation})})())});`
}

function browserStatePageScript(values) {
  const config = JSON.stringify(values)
  const seedWriters = values.mode === "seed" ? `
        const writeDatabase=async()=>{const database=await openDatabase();if(!database.objectStoreNames.contains(config.store)){database.close();return false}await new Promise((resolve,reject)=>{const transaction=database.transaction(config.store,"readwrite");transaction.objectStore(config.store).put(config.databaseValue,config.databaseKey);transaction.oncomplete=resolve;transaction.onerror=()=>reject(transaction.error||new Error("database write failed"))});database.close();return true};
        const writeCache=async()=>{const storage=await caches.open(config.cacheName);await storage.put(config.cacheKey,new Response(config.cacheValue));return true};` : ""
  const phase = values.mode === "seed" ? `
              const initial=await fetch(config.cookieUrl,{credentials:"same-origin",cache:"no-store"});
              const authenticated=initial.status===204?await fetch(config.cookieUrl,{credentials:"same-origin",cache:"no-store"}):initial;
              const authenticatedText=authenticated.ok?await authenticated.text():"";
              state.cookie=authenticated.ok&&authenticatedText==="ROOM_BROWSER_AUTH=PASS";state.auth=state.cookie;
              localStorage.setItem(config.localKey,config.localValue);state.localStorage=localStorage.getItem(config.localKey)===config.localValue;
              state.indexedDB=await writeDatabase();state.cacheStorage=await writeCache();
              await navigator.serviceWorker.register(config.serviceWorkerUrl,{scope:"/"});
              state.serviceWorker=await checkWorker(await navigator.serviceWorker.ready);` : `
              const authenticated=await fetch(config.cookieUrl,{credentials:"same-origin",cache:"no-store"});
              state.cookie=authenticated.ok&&await authenticated.text()==="ROOM_BROWSER_AUTH=PASS";state.auth=state.cookie;
              state.localStorage=localStorage.getItem(config.localKey)===config.localValue;
              state.indexedDB=await readDatabase();state.cacheStorage=await readCache();
              const registration=await navigator.serviceWorker.getRegistration("/");
              state.serviceWorker=await checkWorker(registration);`
  const seedComplete = values.mode === "seed"
    ? `if(Object.values(state).every(Boolean)){const response=await fetch(config.seedCompleteUrl,{method:"POST",credentials:"same-origin",cache:"no-store"});if(!response.ok)throw new Error("state fixture did not enter verification mode")}`
    : ""
  return `
      (()=>{
        const config=${config};
        const state={cookie:false,auth:false,localStorage:false,indexedDB:false,cacheStorage:false,serviceWorker:false};
        const report=document.querySelector("#room-browser-state");
        const render=()=>{report.textContent="ROOM_BROWSER_STATE "+Object.entries(state).map(([key,value])=>key+"="+(value?"PASS":"FAIL")).join(" ")};
        const requestValue=request=>new Promise((resolve,reject)=>{request.onsuccess=()=>resolve(request.result);request.onerror=()=>reject(request.error||new Error("request failed"))});
        const openDatabase=()=>new Promise((resolve,reject)=>{const request=indexedDB.open(config.database,1);request.onupgradeneeded=()=>{if(config.mode==="seed")request.result.createObjectStore(config.store);else request.transaction.abort()};request.onsuccess=()=>resolve(request.result);request.onerror=()=>{if(config.mode==="verify"&&request.error?.name==="AbortError")resolve(null);else reject(request.error||new Error("database open failed"))}});
        const readDatabase=async()=>{const database=await openDatabase();if(!database||!database.objectStoreNames.contains(config.store)){database?.close();return false}const transaction=database.transaction(config.store,"readonly");const value=await requestValue(transaction.objectStore(config.store).get(config.databaseKey));database.close();return value===config.databaseValue};
        const readCache=async()=>{if(!(await caches.keys()).includes(config.cacheName))return false;const response=await (await caches.open(config.cacheName)).match(config.cacheKey);return Boolean(response)&&await response.text()===config.cacheValue};
        ${seedWriters}
        const checkWorker=async registration=>new Promise(resolve=>{const worker=registration?.active;if(!worker){resolve(false);return}const channel=new MessageChannel();const timer=setTimeout(()=>resolve(false),10000);channel.port1.onmessage=event=>{clearTimeout(timer);resolve(event.data?.ok===true)};worker.postMessage({type:"chariox-room-state-check"},[channel.port2])});
        const check=async()=>{
          try{
            ${phase}
          }catch{for(const key of Object.keys(state))state[key]=false}
              ${seedComplete}
              render();
        };
        check();
      })();`
}
