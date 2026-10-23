# Local agent HTTP trigger drill (MP-08 / MP-10 / MP-11)

Build the kernel and native CLI from the same checkout, build `@chariox/server`,
and run a kernel with a disposable `CHARIOX_HOME`. Set `CHARIOX_CLI_BIN` to that
checkout's native CLI. Use a product-linked provider account and the validation
model appropriate to it. Never copy credentials into the repository.

1. In the real web Room, complete a normal turn. Open the agent's plus menu,
   choose **HTTP trigger**, enter a prompt and choose QUEUE. Add the trigger.
   Verify that no workflow or second agent is created and that the returned
   address starts with `http://127.0.0.1:`.
2. Open that address on the kernel's computer. The earlier turn must be visible.
   Expand a tool row and a past turn; check Markdown and code rendering. Repeat
   at 1440 and 400 pixels, DPR 1 and 2, with screen recording.
3. On the same computer, set `TRIGGER_URL` to that local address, then run:

   ```sh
   curl --fail-with-body -H 'Content-Type: application/json' \
     --data '{"prompt":"State the repository name. End with LOCAL_HTTP_DRILL."}' \
     "$TRIGGER_URL"
   ```

   Expect HTTP 202, one firing ID, one real provider turn, and its answer in both
   the local page and original Room. A browser GET with `?prompt=...` invokes too;
   normal history GET, HEAD and prefetch do not invoke. JSON input stays quoted
   external data under the trigger's instruction.
4. Keep a real turn busy, submit two requests, and confirm QUEUE preserves both
   in order. Switch the trigger to STEER in the real client and verify that a
   real active turn receives its input. A rejected steer remains queued.
5. Stop the agent with the normal client control. Both page and invocation must
   return HTTP 503 and show **agent offline**, without queueing a turn. Restart
   the agent and verify recovery. Disabled triggers return 403.
6. Restart the kernel: the same URL must recover. Remove the trigger normally:
   its listener must retire. Probe the kernel's non-loopback address from another
   machine: this ingress must not be reachable or published through the relay.

Record source commits, binary hashes, resolved model, actual turns, screenshots,
console output, exact HTTP statuses and cleanup outside the repository. These
functional cells do not establish latency gates or multi-hour stability. Report
those separately; unit/server fixtures never substitute for this real drill.
