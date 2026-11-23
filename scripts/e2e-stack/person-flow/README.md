# Person-flow drill (PCD-08)

`consent-app.mjs` is a loopback consent page of a test OAuth app we own ("Test Notes wants to connect"). It
gives the live page card a step only the owner can finish, with no real OAuth provider and no real CAPTCHA:

1. The page shows Allow and Deny. Allow reveals a confirmation form.
2. The code the form asks for goes only to `GET /owner-phone` (Bearer owner token, held by the drill harness
   as the person) and is never written to `/status`, the page, or any log.
3. Three wrong codes lock the grant. A correct code moves the tab to `/callback`, which is the step the
   card's browser region follows to close itself.

`startConsentApp()` returns `{origin, ownerToken, close}`. The agent is told to raise one `request_popup`
(`wait:false`, `view.browser_region` on the consent box) and not to touch the page; the person completes it
inside the card from any web client. Check the agent's own tool trace for `open`, `snapshot` and `state` only.

Run the fixture tests with `node --test scripts/e2e-stack/person-flow/consent-app.test.mjs`.
