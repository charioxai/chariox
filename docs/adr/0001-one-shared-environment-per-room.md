# One shared environment per Room

Each Room owns one default browser and graphical Environment under kernel authority. Users and agents share its tabs, browser profile, desktop, viewport, input arbitration, history, and saved state. We rejected per-agent browsers and client-owned viewer sessions because they split authentication and state, make human takeover ambiguous, and let Web or TUI clients become accidental runtime authorities. A user may explicitly create another Environment later, but Chariox must never create one merely because another agent or client attaches.
