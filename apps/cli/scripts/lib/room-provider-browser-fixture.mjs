import { roomBrowserClickFixtureScript } from "./room-browser-click-fixture.mjs"

const formContents = '<label>Browser sample <input name="browser_sample"></label><button type="submit">Submit Browser form</button>'
const formMarkup = `<form method="get" action="/click" target="_top">${formContents}</form>`

// The form result is rendered by the fixture server after navigation. Merely
// clicking or filling an input cannot produce the acceptance marker.
export function roomProviderBrowserFixture(options, requestUrl) {
  if (options?.mode !== "browser") return { script: "", initialClicks: 0 }
  if (options.browserTask !== "form") return { initialClicks: 0, script: roomBrowserClickFixtureScript }
  const accepted = new URL(requestUrl, "http://fixture.invalid").searchParams.get("browser_sample") === "Chariox form sample"
  if (!accepted && options.browserLayout === "nested-frame") {
    const escaped = formMarkup.replaceAll("&", "&amp;").replaceAll('"', "&quot;").replaceAll("<", "&lt;")
    const inner = `<iframe title="Inner form frame" style="width:650px;height:70px" srcdoc="${escaped}"></iframe>`
    return { initialClicks: 0, script: `
      const outer=document.createElement("iframe");
      outer.id="browser-action-target";
      outer.title="Outer form frame";
      outer.style.width="700px";outer.style.height="110px";
      outer.srcdoc=${JSON.stringify(inner)};
      document.body.append(outer);` }
  }
  if (!accepted && options.browserLayout === "shadow-root") return { initialClicks: 0, script: `
    const host=document.createElement("div");
    host.id="browser-action-target";
    host.attachShadow({mode:"open"}).innerHTML=${JSON.stringify(formMarkup)};
    document.body.append(host);` }
  return { initialClicks: accepted ? 1 : 0, script: `
    const browserForm=document.createElement("form");
    browserForm.id="browser-action-target";
    browserForm.method="get";
    browserForm.action="/click";
    browserForm.innerHTML=${JSON.stringify(formContents)};
    document.body.append(browserForm);
    ${accepted ? `const accepted=document.createElement("div");
    accepted.textContent="BROWSER_FORM_ACCEPTED";
    document.querySelector("main").append(accepted);
    document.querySelector("#state").textContent="POINTER_CLICK_COUNT=1";
    document.body.style.background="#69d391";` : ""}` }
}
