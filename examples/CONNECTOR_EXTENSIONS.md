# Connector Extensions

Connector extensions expose external systems to agents as runtime tools. Chariox core does not implement service protocols directly. A connector uses an adapter, and the adapter executable owns the protocol-specific work.

## Layout

User-registered connector files are copied to:

```text
~/.chariox/connectors/definitions/
```

User-registered adapters are copied to:

```text
~/.chariox/connectors/adapters/
```

Package-managed Chariox installs can also ship adapters from the install resource directory. Those appear in `/connector adapter list` together with user-registered adapters.
The adapter implementations live outside the kernel package. The kernel does not link adapter-specific code or dependencies; it only discovers adapter manifests and launches adapter commands.
The shipped adapter package currently includes `http`, `graphql`, `grpc`, `postgres`, and `mysql`.

## 1. Register An Adapter

An adapter is an executable that speaks `chariox-connector-adapter-v2` over JSON lines on stdin/stdout.

```yaml
kind: connector_adapter
name: http
version: 0.1.0
adapter_protocol: chariox-connector-adapter-v2
command: /path/to/chariox-adapter-http
description: HTTP adapter for Chariox connectors.
```

If `command` has no path separators, Chariox launches it through the process `PATH`.
Use `./adapter.py`, `./adapter.mjs`, or another relative path when the executable should be copied with the adapter directory.

Register a user adapter:

```text
/connector adapter register /path/to/adapter.yaml
```

Useful commands:

```text
/connector adapter list
/connector adapter show http
/connector adapter remove custom_adapter
```

## 2. Store A Secret

Store the secret value in the Chariox vault. The value is never sent to the model.

```sh
credential set google-maps-prod
```

Or from the TUI:

```text
/credential set google-maps-prod
```

## 3. Register A Credential

Create a credential YAML file. `source.key` is the vault key, not the secret value.

```yaml
id: google-maps
description: Google Maps API key
source:
  type: vault
  key: google-maps-prod
allowed_hosts:
  - maps.googleapis.com
allowed_uses:
  - connector
injection:
  kind: query
  name: key
```

Register it:

```text
/credential register /path/to/google-maps-credential.yaml
```

## 4. Register A Connector

A connector is the user-facing integration. It chooses an adapter and defines the operations available to agents.

```yaml
kind: connector
name: google_maps
description: Google Maps geocoding connector.
adapter: http
credential:
  required: true
timeout_ms: 30000
max_response_bytes: 1048576
operations:
  - name: geocode
    description: Convert an address into coordinates.
    safety: read
    input_schema:
      type: object
      required: [address]
      properties:
        address:
          type: string
      additionalProperties: false
    config:
      base_url: https://maps.googleapis.com
      method: GET
      path: /maps/api/geocode/json
      query:
        address: "{{address}}"
```

Chariox treats `config` as opaque data. The `http` adapter validates and executes this config.

Register it:

```text
/connector register /path/to/google-maps.yaml
```

Registration fails if common connector validation fails, the adapter is missing, or the adapter rejects the operation configs.

## 5. Test And Grant

```text
/connector doctor google_maps --credential google-maps
/connector test google_maps geocode --credential google-maps --input '{"address":"1600 Amphitheatre Parkway, Mountain View, CA"}'
/connector grant agent-1 google_maps --credential google-maps --allow read
```

The agent receives a tool named `google_maps_geocode`.

Safety levels:

```text
read
write
destructive
```

Grant with `--allow write` or `--allow destructive` only when the agent should receive those operations.

## Adapter Protocol

Validate request:

```json
{
  "id": "validate-1",
  "type": "validate",
  "connector": "google_maps",
  "operations": [
    {
      "name": "geocode",
      "config": {
        "base_url": "https://maps.googleapis.com",
        "method": "GET",
        "path": "/maps/api/geocode/json"
      }
    }
  ],
  "timeout_ms": 30000,
  "max_response_bytes": 1048576
}
```

Prepare request:

The kernel sends `prepare` before any secret is resolved. The adapter renders the call plan and declares the target that any credential would be used against.

```json
{
  "id": "prepare-1",
  "type": "prepare",
  "connector": "google_maps",
  "operation": "geocode",
  "arguments": { "address": "1600 Amphitheatre Parkway" },
  "config": {
    "base_url": "https://maps.googleapis.com",
    "method": "GET",
    "path": "/maps/api/geocode/json",
    "query": { "address": "{{address}}" }
  },
  "timeout_ms": 30000,
  "max_response_bytes": 1048576
}
```

Prepare response:

```json
{
  "id": "prepare-1",
  "ok": true,
  "result": {
    "credential_targets": [
      { "kind": "host", "host": "maps.googleapis.com" }
    ],
    "prepared_config": {
      "base_url": "https://maps.googleapis.com",
      "method": "GET",
      "path": "/maps/api/geocode/json",
      "query": { "address": "1600 Amphitheatre Parkway" }
    }
  }
}
```

Chariox checks the declared target against the credential policy. Only then does it resolve the secret and send the call request.

Call request:

```json
{
  "id": "call-1",
  "type": "call",
  "connector": "google_maps",
  "operation": "geocode",
  "arguments": null,
  "config": {
    "base_url": "https://maps.googleapis.com",
    "method": "GET",
    "path": "/maps/api/geocode/json",
    "query": { "address": "1600 Amphitheatre Parkway" }
  },
  "credential": {
    "id": "google-maps",
    "secret": "...",
    "injection": { "kind": "query", "name": "key" },
    "allowed_hosts": ["maps.googleapis.com"]
  },
  "timeout_ms": 30000,
  "max_response_bytes": 1048576
}
```

Response:

```json
{
  "id": "call-1",
  "ok": true,
  "result": {
    "status": 200,
    "body_json": {}
  }
}
```

Failure:

```json
{
  "id": "call-1",
  "ok": false,
  "error": "request failed"
}
```

Adapters are trusted local code. If a connector is granted a credential, Chariox resolves the secret from the vault only after the adapter declares an allowed credential target, and passes the secret to the adapter process, never to the model.

## Built-In Adapter Examples

- HTTP: `examples/connectors/http/`
- GitHub REST over HTTP: `examples/connectors/github/`
- GitHub GraphQL: `examples/connectors/graphql/`
- gRPC: `examples/connectors/grpc/`
- Postgres: `examples/connectors/postgres/`
- MySQL: `examples/connectors/mysql/`
