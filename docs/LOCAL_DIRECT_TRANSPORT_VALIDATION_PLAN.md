# MP-08 / MP-10 / MP-11 automatic direct transport drills

Protocol 520 provides authenticated browser and TUI direct carriers with
short, key-bound grants and renewal.
Pair with chariox-cloud #307; OSS #916 owns runtime authority and TUI projection.
These scenarios validate the same kernel session and command/subscription paths.
A loopback development run is supplementary; hosted live acceptance still uses
real accounts/providers, the hosted WSS relay, DPR 1 and 2, realistic network
conditions and the required duration.

| Scenario | User flow and expected observation |
| --- | --- |
| LD-01 / MP-08 | Open the real web terminal for a same-machine kernel with a fresh profile. A fresh Cloud observation of an owned kernel on the browser's public IP, or a previous authenticated local proof, permits automatic probing. Before a browser-owned permission prompt, a visible Chariox explanation says why access is needed. Direct admission still requires the ordinary key-bound grant and proof. No application Allow button. |
| LD-02 / MP-08 | Reload, suspend/resume, and restart the kernel. The same session remains usable; ordinary local failure falls back to relay and direct is restored automatically after cooldown. |
| LD-03 / MP-08 | Select a remote, managed or slice kernel without same-machine discovery/proof. No Local/Relay indicator or local offer. |
| LD-04 / MP-08 / MP-11 | Block local sockets before they open for an online, previously proven local kernel. Relay remains usable; one notice outside agent content suggests Chrome or Firefox. Reload does not repeat the notice. Keep Chromium's pending permission request pending rather than reporting an error. |
| LD-05 / MP-11 | Replace the loopback listener. No runtime request reaches it unless it proves the pinned kernel key and redeems the bound-client grant. Failed proof blocks local attempts. |
| LD-06 / MP-08 / MP-11 | Real detached and native-provider TUIs automatically admit direct for a fresh, id-addressed local target at protocol 520 after kernel-key proof and a short terminal grant; renew through the same terminal contract. Show the carrier indicator only for a fresh local discovery match. Older, alias-only and undiscovered targets keep the relay. Never select discovered TCP solely on its public kernel id. |
| LD-LNA / MP-08 / MP-10 / MP-11 | Hosted Chrome stable, fresh profiles, 1440×900 at DPR 1 and 2: a device with no positive locality evidence sends no loopback request and shows no native permission prompt; a same-machine owned kernel prompts at most once with the explanation already visible and connects directly after Allow; Block keeps relay usable and reload does not probe or prompt again. A browser site-settings grant permits recovery. Record screen video including browser chrome, per-step screenshots, permission state, and actual carrier. Repeat available Safari/Firefox flows without changing their automatic fallback. |

MP-10 evidence records exact source identities, built clients/kernel/relay,
per-step screenshots and allowlisted event metadata, resource samples, and
complete cleanup. A first Safari visit has no prior locality proof; a trusted
browser-machine association or fresh same-public-IP hint is needed to distinguish
it from a remote target before the browser permits a connection. The network hint
permits a probe and does not prove locality; NAT may produce false positives, while
VPNs or differing address families may keep a first visit on relay. A denied
origin remains on relay until the browser reports a site-settings grant.
