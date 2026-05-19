# Script Extensions

Script extensions let an Arroba agent call a local Python or TypeScript function as a tool. The user script only needs to define one callable `run` function plus a `test_run` validation function. Arroba handles tool input parsing, process execution, JSON serialization, and returning the result to the model.

Scripts must exist on the machine hosting the agent. Arroba does not install Python or Node packages for you in v1; point Arroba at the environment you want it to use.

## Python

Create a script anywhere on the agent-hosting machine:

```python
# examples/vector_lookup.py

def run(query: str, limit: int = 3) -> list[str]:
    """Return matching document ids for a query."""
    return [f"{query}-doc-{index}" for index in range(limit)]

def test_run() -> None:
    assert run("alpha", 2) == ["alpha-doc-0", "alpha-doc-1"]
```

Register the Python environment:

```text
/env register py-default --python /usr/bin/python3
```

Validate and register the script:

```text
/script validate /absolute/path/to/examples/vector_lookup.py --env py-default --name vector_lookup
/script register /absolute/path/to/examples/vector_lookup.py --env py-default --name vector_lookup
```

Grant it to an agent:

```text
/script grant agent-1 vector_lookup --env py-default
```

The agent can now call `vector_lookup` with JSON input such as:

```json
{"query":"alpha","limit":2}
```

The model receives the returned value directly:

```json
["alpha-doc-0","alpha-doc-1"]
```

## Python With Packages

Use whichever Python executable has the packages installed:

```sh
python3 -m venv .venv
. .venv/bin/activate
pip install requests
```

Example script:

```python
# examples/package_lookup.py
import requests

def run(url: str) -> dict[str, object]:
    """Fetch a URL and return basic response metadata."""
    response = requests.get(url, timeout=10)
    return {
        "status": response.status_code,
        "content_type": response.headers.get("content-type"),
        "bytes": len(response.content),
    }

def test_run() -> None:
    result = run("https://example.com")
    assert isinstance(result["status"], int)
```

Register the venv Python:

```text
/env register py-requests --python /absolute/path/to/.venv/bin/python
/script register /absolute/path/to/examples/package_lookup.py --env py-requests --name package_lookup
/script grant agent-1 package_lookup --env py-requests
```

## TypeScript

Create a package root for Node dependencies:

```sh
mkdir -p .arroba-script-envs/ts-default
cd .arroba-script-envs/ts-default
npm init -y
npm install tsx
```

Create the script:

```ts
// examples/account_lookup.ts

/**
 * Return account facts from a deterministic local fixture.
 */
export function run(accountId: string, multiplier: number = 3): Record<string, unknown> {
  return {
    accountId,
    score: multiplier * 7,
    records: [
      { id: "rec-1", label: "primary" },
      { id: "rec-2", label: "secondary" },
    ],
  }
}

export function test_run(): void {
  const result = run("acct-demo", 2)
  if (result.score !== 14) throw new Error("bad score")
}
```

Register Node with the package root that has `tsx`:

```text
/env register node-default --node /opt/homebrew/bin/node --package-root /absolute/path/to/.arroba-script-envs/ts-default
```

Validate, register, and grant:

```text
/script validate /absolute/path/to/examples/account_lookup.ts --env node-default --name account_lookup
/script register /absolute/path/to/examples/account_lookup.ts --env node-default --name account_lookup
/script grant agent-1 account_lookup --env node-default
```

The agent can call `account_lookup` with:

```json
{"accountId":"acct-42","multiplier":3}
```

The model receives:

```json
{
  "accountId": "acct-42",
  "score": 21,
  "records": [
    {"id": "rec-1", "label": "primary"},
    {"id": "rec-2", "label": "secondary"}
  ]
}
```

## TypeScript With Packages

Install packages into the Node package root:

```sh
cd .arroba-script-envs/ts-default
npm install zod
```

Example script:

```ts
// examples/zod_parse.ts
import { z } from "zod"

const Input = z.object({
  values: z.array(z.number()),
})

/**
 * Validate numbers and return summary statistics.
 */
export function run(values: number[]): Record<string, unknown> {
  const parsed = Input.parse({ values })
  return {
    count: parsed.values.length,
    sum: parsed.values.reduce((total, value) => total + value, 0),
  }
}

export function test_run(): void {
  const result = run([1, 2, 3])
  if (result.sum !== 6) throw new Error("bad sum")
}
```

Register and grant it with the same Node environment:

```text
/script register /absolute/path/to/examples/zod_parse.ts --env node-default --name zod_parse
/script grant agent-1 zod_parse --env node-default
```

## Equivalent Extension Commands

`/script grant` is the shortest command for scripts. The generic extension command is equivalent:

```text
/extension grant script agent-1 vector_lookup --env py-default
/extension grants script agent-1
/extension revoke script agent-1 vector_lookup
```

The old MCP and skill commands remain available as aliases for those extension kinds:

```text
/mcp grants agent-1
/skill grants agent-1
```

## Cleanup

Remove a script grant:

```text
/script revoke agent-1 vector_lookup
```

Remove a registered script:

```text
/script remove vector_lookup
```

Remove an environment:

```text
/env remove py-default
```

## User Contract

Keep scripts simple:

- Define exactly one model-callable function named `run`.
- Add full type hints to `run` parameters.
- Add a return type annotation in Python.
- Add a docstring or JSDoc comment to describe the tool to the model.
- Define `test_run` with a toy example. Arroba runs it during validation/registration.
- Return normal JSON-serializable values: strings, numbers, booleans, lists, dicts/objects, or `None`/`null`.
- Do not print JSON to stdout for Arroba.
- Do not write result files for Arroba.
- Do not import an Arroba library.

Arroba's runner serializes the return value and sends it back to the model.
