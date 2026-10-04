// MP-08 / MP-10 / MP-11: selection uses the observed native control only.
export async function fillNativeSelect(connection, sessionId, objectId, action) {
  if (action.append || action.submit || action.expectedDocumentUrl !== null) {
    throw selectionError("browser_selection_invalid", "native select fill replaces one choice; append, submit and secret fill are unsupported");
  }
  const response = await connection.send("Runtime.callFunctionOn", {
    objectId,
    functionDeclaration: selectFunction.toString(),
    arguments: [{ value: action.text }],
    returnByValue: true,
  }, sessionId);
  const outcome = response?.result?.value;
  if (response?.exceptionDetails || outcome?.ok !== true) {
    const reason = outcome?.reason ?? "failed";
    throw selectionError(`browser_selection_${reason}`, `native select choice ${reason}; rediscover the control and its options`);
  }
}

function selectionError(code, message) {
  return Object.assign(new Error(message), { code });
}

export function selectFunction(text) {
  if (!this.isConnected || this.tagName !== "SELECT") return { ok: false, reason: "stale" };
  if (this.matches(":disabled") || this.closest("[inert]") || this.getAttribute("aria-disabled") === "true") return { ok: false, reason: "disabled" };
  const options = Array.from(this.options);
  let matches = options.filter(option => option.value === text);
  if (!matches.length) matches = options.filter(option => option.label.trim() === text);
  if (!matches.length) return { ok: false, reason: "not_found" };
  if (matches.length !== 1) return { ok: false, reason: "ambiguous" };
  const choice = matches[0];
  if (choice.disabled || choice.parentElement?.tagName === "OPTGROUP" && choice.parentElement.disabled) return { ok: false, reason: "disabled" };
  // Do all validation before the first effect; never retry after event delivery.
  const changed = options.some(option => option.selected !== (option === choice));
  if (changed) {
    for (const option of options) option.selected = option === choice;
    const ownerWindow = this.ownerDocument.defaultView;
    this.dispatchEvent(new ownerWindow.Event("input", { bubbles: true, composed: true }));
    this.dispatchEvent(new ownerWindow.Event("change", { bubbles: true }));
  }
  const applied = Array.from(this.options);
  if (!this.isConnected || !applied.includes(choice) || applied.some(option => option.selected !== (option === choice))) {
    return { ok: false, reason: "not_applied" };
  }
  return { ok: true };
}
