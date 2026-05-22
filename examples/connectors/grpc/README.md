# gRPC Connector Example

The `grpc` adapter shells out to `grpcurl`, so `grpcurl` must be installed on the machine hosting the agent.
This keeps the Arroba adapter generic: the connector can use server reflection or provide proto files.

Register the example connector:

```text
/connector register examples/connectors/grpc/connector.yaml
```

If the server does not support reflection, add `import_paths` and `protos`:

```yaml
      import_paths:
        - ./proto
      protos:
        - echo.proto
```

Test and grant:

```text
/connector test grpc_example echo --allow read --input '{"message":"hello"}'
/connector grant <agent> grpc_example --allow read
```
