# Generated load lab traces

Generate a fixture set outside the checkout. Use the same seed to reproduce the exact JSONL files.

```sh
node scripts/drills/loadlab/generate.cjs s1 42 /absolute/path/to/traces
```

The generator also accepts `s2` and `s3`. Start a private drill kernel with `CHARIOX_PROVIDER_DEV_STUB=1`, `CHARIOX_ROOM_AGENT_TOOLS=1`, and `CHARIOX_LOADLAB_TRACE_DIR` set to the absolute trace directory. The kernel launches a replay process for provider `dev-stub`, model `trace`. It passes the agent's alias to the process, which loads the corresponding `<alias>.jsonl`. The process reads only the generated fixture directory.

Each file starts with `{"v":1,"source":"generator","seed":42,"agent":"orch","notes":"..."}`. A `prompt` step opens each turn at zero milliseconds and an `end` step closes it with `completed`, `cancelled`, or `error`. Each step's `t` is measured from the start of its turn. The parser checks the full file before replay. The `orch.jsonl` fixture shows a turn with reasoning, text, and a synthetic tool call and result.

A prompt submitted to the kernel advances the trace by one turn. The `prompt.chars` field controls the filler used by the seeder. A `spawn` step starts a real child and submits the first synthetic prompt from that child's trace. The child trace filename must match its alias. A `permission` step waits for the kernel's normal native permission decision. The terminal and notification steps use the lab Room's actual terminal and notification services.

S1's `seedS1` takes the lab client RPC, Room and attachment IDs, and the same fixture map used to start the kernel. It starts `orch`, waits for the two-level descendant tree, submits the remaining turns for the 22 history agents, and returns their observed history sizes and the descendant tree when it has the requested states.

```sh
node --test scripts/drills/loadlab/trace.test.cjs
cargo test -p chariox-kernel --test dev_trace
cargo test -p chariox-kernel --test dev_trace trace_seeds_s1_through_the_real_kernel -- --ignored --nocapture
```
