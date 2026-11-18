import type { Mirror2Tile } from './browser-mirror2-types.js'

// A closed shadow root's content can paint outside its host's box (a consent banner in a 0x0 host with fixed content).
// Its still is then drawn by the viewer's own child of the host, which also takes the pointer for the host.
const INK = 'data-chariox-region-ink'
export type TileBox = Pick<Mirror2Tile, 'x' | 'y' | 'width' | 'height'>

/** Whether a still lies outside its element's box by more than the kernel's pixel rounding. */
export const tileOutsideBox = (tile: TileBox, [width, height]: [number, number]) =>
  tile.x < -1 || tile.y < -1 || tile.x + tile.width > width + 1 || tile.y + tile.height > height + 1

export const isTileInk = (node: Node) => (node as Element).hasAttribute?.(INK) === true

/** Draws the still at its place relative to the element, or removes the drawing (no url). */
export function paintTileInk(element: HTMLElement, tile: TileBox, url: string | null): void {
  let ink = Array.from(element.children).find(isTileInk) as HTMLElement | undefined
  if (!url) { ink?.remove(); return }
  if (!ink) { ink = element.ownerDocument.createElement('div'); ink.setAttribute(INK, ''); element.append(ink) }
  const host = element.style, view = element.ownerDocument.defaultView
  host.setProperty('overflow', 'visible', 'important'); host.removeProperty('background-image')
  if (view?.getComputedStyle(element).position === 'static') host.setProperty('position', 'relative', 'important')
  const style = ink.style
  for (const [name, value] of Object.entries({ all: 'initial', position: 'absolute', display: 'block', left: `${tile.x}px`, top: `${tile.y}px`, width: `${tile.width}px`, height: `${tile.height}px`,
    'background-image': `url("${url}")`, 'background-repeat': 'no-repeat', 'background-size': '100% 100%' })) style.setProperty(name, value, 'important')
}
