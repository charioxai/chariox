# GraphQL Connector Example

This directory includes a no-auth public GraphQL example and an authenticated GitHub GraphQL example.

The `graphql` adapter sends a POST request with `query`, optional `operationName`, and `variables`.

Public example:

```text
/connector register examples/connectors/graphql/countries.yaml
/connector test countries_graphql country --allow read --input '{"code":"DE"}'
/connector grant <agent> countries_graphql --allow read
```

GitHub example:

```text
/connector register examples/connectors/graphql/github_graphql.yaml
```

Test:

```text
/connector test github_graphql viewer --credential github-api --allow read --input '{}'
```

The example reuses the GitHub credential from `examples/connectors/github/credential.yaml`.
