// MP-08 / MP-09 / MP-10 / MP-11: put one URL in the tab that is in FRONT of the kernel browser, so a person only has to
// act on the page: node front-page.mjs <url>. The owner API has no tab activate; navigating the active tab never
// moves the person's view. Also lists the open tabs (title and URL only) so a card can name the page.
import {connect} from './rpc.mjs'

const url = process.argv[2]
const peer = await connect()
const browser = async command => (await peer.send({KernelBrowser: {command}})).KernelBrowser.result
const state = await browser({op: 'state'})
if (url) await browser({op: 'navigate', tab_id: state.active_tab_id, generation: state.generation, url})
const after = url ? await browser({op: 'state'}) : state
for (const tab of after.tabs) console.log(tab.tab_id === after.active_tab_id ? '*' : ' ', tab.tab_id.slice(-8), tab.title.slice(0, 40), tab.url.slice(0, 90))
peer.close()
