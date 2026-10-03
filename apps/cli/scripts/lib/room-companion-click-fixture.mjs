export function roomCompanionClickFixture(realProvider) {
  return {
    path: "/click?room-companion-reset=1",
    readyMarker: "POINTER_CLICK_READY",
    // Provider click/form completion contributes one click before the human
    // takes over. Office tasks have a separate physical-effect contract.
    pointerClickExpectedCount: realProvider && realProvider.computerTask !== "office" ? 2 : 1,
  }
}

// This counter survives ordinary page reloads for browser persistence drills.
export function roomClickFixtureCounterScript({ storageKey, initialClicks, requestUrl }) {
  const reset = new URL(requestUrl, "http://fixture.invalid").searchParams.get("room-companion-reset") === "1"
  return `
    const clickCountStorageKey=${JSON.stringify(storageKey)};
    if(${reset}){
      localStorage.removeItem(clickCountStorageKey);
      const restoredUrl=new URL(location.href);
      restoredUrl.searchParams.delete("room-companion-reset");
      history.replaceState(history.state,"",restoredUrl.href);
    }
    const storedClickCount=localStorage.getItem(clickCountStorageKey);
    let clicks=${initialClicks};
    if(storedClickCount!==null){const restoredCount=Number(storedClickCount);if(Number.isSafeInteger(restoredCount)&&restoredCount>=0){clicks=restoredCount;document.querySelector("#state").textContent="POINTER_CLICK_COUNT="+clicks}}
    document.addEventListener("click",(event)=>{if(event.target.closest("[data-web-gesture]"))return;clicks+=1;localStorage.setItem(clickCountStorageKey,String(clicks));document.body.style.background="#69d391";document.querySelector("#state").textContent="POINTER_CLICK_COUNT="+clicks})`
}
