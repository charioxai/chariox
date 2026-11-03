// The reference App accepts native diagram actions and Excalidraw JSON.
// Excalidraw catches this and pastes Mermaid source as ordinary text.
// Keep TTDDialogTrigger unmounted; that optional dialog needs the full parser.
export async function parseMermaidToExcalidraw() {
  throw new Error('Mermaid conversion is not included in this App. Paste ordinary text or open an Excalidraw diagram.');
}
