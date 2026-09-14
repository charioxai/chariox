import { roomBrowserClickFixtureScript } from "./room-browser-click-fixture.mjs"

const baseFormContents = '<label>Browser sample <input name="browser_sample"></label><button type="submit">Submit Browser form</button>'
const replacementControls = `<input type="hidden" name="browser_replaced" value="0">
  <input type="hidden" name="browser_stale_safe" value="1">
  <button type="button" onclick="
    const form=this.form,old=form.elements.browser_sample;
    const replacement=old.cloneNode(false);
    old.oninput=()=>{form.elements.browser_stale_safe.value='0'};
    replacement.oninput=()=>{if(replacement.value.includes('STALE'))form.elements.browser_stale_safe.value='0'};
    old.replaceWith(replacement);
    form.elements.browser_replaced.value='1';this.disabled=true;
  ">Replace Browser field</button>`

// The form result is rendered by the fixture server after navigation. Merely
// clicking or filling an input cannot produce the acceptance marker.
export function roomProviderBrowserFixture(options, requestUrl) {
  if (options?.mode !== "browser") return { script: "", initialClicks: 0 }
  if (options.browserTask !== "form") return { initialClicks: 0, script: roomBrowserClickFixtureScript }
  const recovery = options.browserMutation === "replace-field"
  const formContents = baseFormContents + (recovery ? replacementControls : "")
  const formMarkup = `<form method="get" action="/click" target="_top">${formContents}</form>`
  const params = new URL(requestUrl, "http://fixture.invalid").searchParams
  const accepted = params.get("browser_sample") === "Chariox form sample"
    && (!recovery || (params.get("browser_replaced") === "1" && params.get("browser_stale_safe") === "1"))
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
    ${recovery ? 'const recovered=document.createElement("div");recovered.textContent="BROWSER_STALE_RECOVERY_ACCEPTED";document.querySelector("main").append(recovered);' : ""}
    document.querySelector("#state").textContent="POINTER_CLICK_COUNT=1";
    document.body.style.background="#69d391";` : ""}` }
}
