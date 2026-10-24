export const sharedTooltipStyles = `
.ds-tooltip-host {
  position: relative;
  display: inline-flex;
  align-items: center;
}

.ds-tooltip {
  position: absolute;
  z-index: var(--z-dropdown);
  left: 50%;
  bottom: calc(100% + 6px);
  transform: translateX(-50%);
  max-width: 220px;
  padding: 3px var(--space-2);
  border: 1px solid color-mix(in srgb, var(--line-strong) 72%, transparent);
  border-radius: var(--radius-sm);
  background: color-mix(in srgb, var(--surface-raised) 96%, transparent);
  box-shadow: 0 4px 12px rgb(0 0 0 / 28%);
  color: var(--text);
  font-family: var(--font-display);
  font-size: var(--text-2xs);
  font-weight: 400;
  line-height: 1.4;
  white-space: nowrap;
  pointer-events: none;
  opacity: 0;
  transition: opacity var(--motion-fast);
}

.ds-tooltip-host[data-side="bottom"] > .ds-tooltip {
  top: calc(100% + 6px);
  bottom: auto;
}

.ds-tooltip-host[data-side="left"] > .ds-tooltip {
  right: calc(100% + 6px);
  left: auto;
  bottom: 50%;
  transform: translateY(50%);
}

.ds-tooltip-host[data-side="right"] > .ds-tooltip {
  left: calc(100% + 6px);
  bottom: 50%;
  transform: translateY(50%);
}

/* Show on hover or when child is focused */
.ds-tooltip-host:hover > .ds-tooltip,
.ds-tooltip-host:focus-within > .ds-tooltip {
  opacity: 1;
}

.ds-tooltip-host[data-tooltip-delay="slow"] > .ds-tooltip {
  transition-delay: 0ms;
}

.ds-tooltip-host[data-tooltip-delay="slow"]:hover > .ds-tooltip,
.ds-tooltip-host[data-tooltip-delay="slow"]:focus-within > .ds-tooltip {
  transition-delay: 1200ms;
}

.ds-tooltip-host[data-tooltip-delay="long"] > .ds-tooltip {
  transition-delay: 0ms;
}

.ds-tooltip-host[data-tooltip-delay="long"]:hover > .ds-tooltip,
.ds-tooltip-host[data-tooltip-delay="long"]:focus-within > .ds-tooltip {
  transition-delay: 2400ms;
}

.ds-tooltip-host[data-tooltip-trigger="hover"]:focus-within:not(:hover) > .ds-tooltip {
  opacity: 0;
}

`
