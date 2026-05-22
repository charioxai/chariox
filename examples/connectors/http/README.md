# HTTP Connector Example

This example uses the shipped `http` adapter against `httpbin.org`, so it does not require an account or credential.

Register:

```text
/connector register examples/connectors/http/httpbin.yaml
```

Test:

```text
/connector test httpbin get --allow read --input '{"topic":"arroba"}'
```

Grant:

```text
/connector grant <agent> httpbin --allow read
```
