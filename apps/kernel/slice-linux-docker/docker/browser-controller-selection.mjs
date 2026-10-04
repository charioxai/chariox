// MP-08 / MP-10 / MP-11: native fills use the observed control only.
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

// MP-08/MP-10/MP-11: segmented native controls cannot consume Input.insertText.
export async function fillNativeTemporal(connection, sessionId, objectId, action) {
  if (action.append || action.expectedDocumentUrl !== null) {
    throw selectionError("browser_fill_invalid", "native date/time fill replaces the complete value; append and secret fill are unsupported");
  }
  const response = await connection.send("Runtime.callFunctionOn", {
    objectId,
    functionDeclaration: temporalFunction.toString(),
    arguments: [{ value: action.text }],
    returnByValue: true,
  }, sessionId);
  const outcome = response?.result?.value;
  if (response?.exceptionDetails || outcome?.ok !== true) {
    const reason = outcome?.reason ?? "failed";
    throw selectionError(`browser_fill_${reason}`, `native date/time fill ${reason}; capture a fresh snapshot before retrying`);
  }
}

export function temporalFunction(text) {
  const ownerWindow = this.ownerDocument?.defaultView;
  const prototype = ownerWindow?.HTMLInputElement?.prototype;
  const types = ["date", "time", "datetime-local", "month", "week"];
  if (!this.isConnected) return { ok: false, reason: "stale" };
  if (!prototype || !prototype.isPrototypeOf(this) || !types.includes(this.type)) return { ok: false, reason: "not_editable" };
  if (this.matches(":disabled") || this.closest("[inert]") || this.getAttribute("aria-disabled") === "true") return { ok: false, reason: "disabled" };
  if (this.readOnly || this.getAttribute("aria-readonly") === "true") return { ok: false, reason: "not_editable" };
  const nativeValue = Object.getOwnPropertyDescriptor(prototype, "value");
  // Chromium validates syntax on a detached control before any visible effect.
  const probe = this.ownerDocument.createElement("input");
  probe.type = this.type;
  nativeValue.set.call(probe, text);
  const value = nativeValue.get.call(probe);
  if (text !== "" && value === "") return { ok: false, reason: "invalid_value" };
  const type = this.type;
  if (nativeValue.get.call(this) !== value) {
    // Bypass framework instance value tracking, then notify it exactly once.
    nativeValue.set.call(this, value);
    this.dispatchEvent(new ownerWindow.Event("input", { bubbles: true, composed: true }));
    this.dispatchEvent(new ownerWindow.Event("change", { bubbles: true }));
  }
  if (!this.isConnected || this.type !== type || nativeValue.get.call(this) !== value) {
    return { ok: false, reason: "not_applied" };
  }
  return { ok: true };
}
