// MP-08/MP-10/MP-11: native target visibility and geometry below the clients.
export function actionabilityFunction(allowVisiblePoint = false) {
  if (!this.isConnected) return { state: "detached" };
  this.scrollIntoView({ block: "center", inline: "center", behavior: "instant" });
  const ownerDocument = this.ownerDocument;
  const ownerWindow = ownerDocument.defaultView;
  const style = ownerWindow.getComputedStyle(this);
  const rect = this.getBoundingClientRect();
  if (
    style.display === "none" ||
    style.visibility === "hidden" ||
    Number(style.opacity) === 0 ||
    rect.width <= 0 ||
    rect.height <= 0
  ) {
    return { state: "not_visible" };
  }
  if (
    this.disabled ||
    this.matches?.(":disabled") ||
    (this.tagName === "LABEL" && this.control?.matches?.(":disabled")) ||
    this.closest?.("[inert]") ||
    this.getAttribute?.("aria-disabled") === "true"
  ) {
    return { state: "disabled" };
  }
  let localX = rect.left + rect.width / 2;
  let localY = rect.top + rect.height / 2;
  const receivesInput = (x, y) => {
    let hitTarget = ownerDocument.elementFromPoint(x, y);
    while (hitTarget?.shadowRoot?.elementFromPoint) {
      const nestedTarget = hitTarget.shadowRoot.elementFromPoint(x, y);
      if (!nestedTarget || nestedTarget === hitTarget) break;
      hitTarget = nestedTarget;
    }
    let inside = hitTarget === this || this.contains?.(hitTarget);
    let root = hitTarget?.getRootNode?.();
    while (!inside && root?.host) {
      inside = root.host === this || this.contains?.(root.host);
      root = root.host.getRootNode?.();
    }
    return Boolean(hitTarget && inside);
  };
  if (!receivesInput(localX, localY)) {
    // Main-frame clicks can use a bounded interior point verified by native
    // hit testing. Preserve center-bound drags and existing frame transforms.
    let point = null;
    if (allowVisiblePoint && ownerWindow === ownerWindow.top) {
      const left = Math.max(0, rect.left);
      const top = Math.max(0, rect.top);
      const right = Math.min(ownerWindow.innerWidth, rect.right);
      const bottom = Math.min(ownerWindow.innerHeight, rect.bottom);
      if (right > left && bottom > top) {
        const insetX = Math.min(2, (right - left) / 4);
        const insetY = Math.min(2, (bottom - top) / 4);
        const middleX = (left + right) / 2;
        const middleY = (top + bottom) / 2;
        const points = [
          [left + insetX, top + insetY], [right - insetX, top + insetY],
          [left + insetX, bottom - insetY], [right - insetX, bottom - insetY],
          [middleX, top + insetY], [middleX, bottom - insetY],
          [left + insetX, middleY], [right - insetX, middleY],
        ];
        for (const [x, y] of points) {
          if (receivesInput(x, y)) {
            point = [x, y];
            break;
          }
        }
      }
    }
    if (!point) return { state: "obscured" };
    [localX, localY] = point;
  }
  let x = localX;
  let y = localY;
  let currentWindow = ownerWindow;
  let crossOriginFrame = false;
  while (currentWindow && currentWindow !== currentWindow.top) {
    const frameElement = currentWindow.frameElement;
    if (!frameElement) {
      crossOriginFrame = true;
      break;
    }
    const frameRect = frameElement.getBoundingClientRect();
    const frameStyle = frameElement.ownerDocument.defaultView.getComputedStyle(frameElement);
    const paddingLeft = Number.parseFloat(frameStyle.paddingLeft) || 0;
    const paddingTop = Number.parseFloat(frameStyle.paddingTop) || 0;
    x += frameRect.left + frameElement.clientLeft + paddingLeft;
    y += frameRect.top + frameElement.clientTop + paddingTop;
    currentWindow = frameElement.ownerDocument.defaultView;
  }
  const inputType = this.matches?.("input")
    ? String(this.type || "text").toLowerCase()
    : null;
  const editableInput = inputType !== null && ![
    "button",
    "checkbox",
    "color",
    "file",
    "hidden",
    "image",
    "radio",
    "range",
    "reset",
    "submit",
  ].includes(inputType);
  const editable =
    this.isContentEditable ||
    ((editableInput || (this.matches?.("textarea") ?? false)) && !this.readOnly);
  return {
    state: "ready",
    x,
    y,
    width: rect.width,
    height: rect.height,
    editable,
    nativeSelect: this.tagName === "SELECT",
    nativeValue: !this.readOnly && ["date", "time", "datetime-local", "month", "week", "range"].includes(inputType),
    ...(crossOriginFrame ? { crossOriginFrame: true } : {}),
  };
}
