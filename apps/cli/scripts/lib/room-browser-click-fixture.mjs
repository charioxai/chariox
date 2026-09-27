// This is embedded in the physical page. Keep provider acceptance separate
// from the document-wide counter used by later human takeover checks.
export const roomBrowserClickFixtureScript = `
  const browserTarget=document.createElement("button");
  browserTarget.id="browser-action-target";
  browserTarget.textContent="Browser action target";
  document.body.append(browserTarget);
  const browserResult=document.createElement("div");
  document.querySelector("main").append(browserResult);
  browserTarget.addEventListener("click",()=>{browserResult.textContent="BROWSER_CLICK_ACCEPTED"});`
