# GitHub Connector Example

This example gives an agent read access to selected GitHub REST API operations through the Chariox HTTP adapter.
The GitHub token stays in the Chariox vault and is not sent to the model.

## 1. Get A GitHub Token

If GitHub CLI is already authenticated, you can reuse its token:

```sh
gh auth status
gh auth token
```

For a dedicated token, create a GitHub personal access token with only the scopes you need.
For public repository metadata and public issues, no special repository scope is required.
For private repositories, use the narrowest repository read scope available for your token type.

## 2. Store The Token In The Chariox Vault

Store the token under the vault key used by `credential.yaml`:

```text
/credential set github-api-token
```

Paste the token when prompted.

## 3. Register The Credential

```text
/credential register examples/connectors/github/credential.yaml
```

The credential allows connector use only, and only against `api.github.com`.

## 4. Register The Connector

Make sure the HTTP adapter is available:

```text
/connector adapter list
```

Then register the GitHub connector:

```text
/connector register examples/connectors/github/connector.yaml
```

## 5. Test It

```text
/connector test github viewer --credential github-api --allow read --input '{}'
/connector test github repo --credential github-api --allow read --input '{"owner":"mgutierrez09","repo":"chariox"}'
/connector test github list_repo_issues --credential github-api --allow read --input '{"owner":"mgutierrez09","repo":"chariox"}'
```

## 6. Grant It To An Agent

Grant the connector to an agent with read-only safety:

```text
/extension grant connector <agent> github --credential github-api --allow read
```

After that, the model can call:

- `github_viewer`
- `github_repo`
- `github_list_repo_issues`
