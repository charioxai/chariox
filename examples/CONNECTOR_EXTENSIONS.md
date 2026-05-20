# Connector Extensions

Connector extensions expose a configured external API to an agent as runtime tools. V1 supports YAML-defined HTTP connectors, with secrets referenced through Arroba credential records and resolved from the OS vault at call time.

## 1. Store The Secret

Store the secret value in the Arroba vault. From the workspace shell:

```sh
credential set google-maps-prod
```

In the TUI, use the slash command when hidden input support is available:

```text
/credential set google-maps-prod
```

The shell prompts for the value with hidden input. The value is stored in the OS keychain under the Arroba vault service and is never placed in the model context.

## 2. Register A Credential

Create a credential YAML file. The `source.key` is the vault key, not the secret value.

```yaml
id: google-maps
description: Google Maps API key
source:
  type: vault
  key: google-maps-prod
allowed_hosts:
  - maps.googleapis.com
allowed_uses:
  - http
injection:
  kind: query
  name: key
```

Register it:

```text
/credential register /path/to/google-maps-credential.yaml
```

## 3. Register A Connector

Create a connector YAML file:

```yaml
kind: connector
name: google_maps
description: Google Maps geocoding connector.
type: http
base_url: https://maps.googleapis.com
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
    request:
      method: GET
      path: /maps/api/geocode/json
      query:
        address: "{{address}}"
```

Register it:

```text
/connector register /path/to/google-maps-connector.yaml
```

## 4. Test And Grant

Run a metadata/policy check, then test the connector before granting it to an agent:

```text
/connector doctor google_maps --credential google-maps
```

```text
/connector test google_maps geocode --credential google-maps --input '{"address":"1600 Amphitheatre Parkway, Mountain View, CA"}'
```

Grant it to an agent:

```text
/connector grant agent-1 google_maps --credential google-maps --allow read
```

The agent receives a tool named `google_maps_geocode`. If an operation has `safety: write`, grant it with `--allow write`; if it has `safety: destructive`, grant it with `--allow destructive`.

Useful inspection commands:

```text
/credential list
/credential show google-maps
/connector list
/connector show google_maps
/connector grants agent-1
```
