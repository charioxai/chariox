// The form result is rendered by the fixture server after navigation. Merely
// clicking or filling an input cannot produce the acceptance marker.
export function roomProviderBrowserFixture(options, requestUrl) {
  if (options?.mode !== "browser") return { script: "", initialClicks: 0 }
  if (options.browserTask !== "form") return { initialClicks: 0, script: `
    const browserTarget=document.createElement("button");
    browserTarget.id="browser-action-target";
    browserTarget.textContent="Browser action target";
    document.body.append(browserTarget);` }
  const accepted = new URL(requestUrl, "http://fixture.invalid").searchParams.get("browser_sample") === "Chariox form sample"
  return { initialClicks: accepted ? 1 : 0, script: `
    const browserForm=document.createElement("form");
    browserForm.id="browser-action-target";
    browserForm.method="get";
    browserForm.action="/click";
    browserForm.innerHTML='<label>Browser sample <input name="browser_sample"></label><button type="submit">Submit Browser form</button>';
    document.body.append(browserForm);
    ${accepted ? `const accepted=document.createElement("div");
    accepted.textContent="BROWSER_FORM_ACCEPTED";
    document.querySelector("main").append(accepted);
    document.querySelector("#state").textContent="POINTER_CLICK_COUNT=1";
    document.body.style.background="#69d391";` : ""}` }
}
