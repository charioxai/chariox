# Postgres Connector Example

The `postgres` adapter runs SQL with positional `$1`, `$2`, ... parameters and returns rows as JSON.

Credentials can use `basic` injection, where `username` is the database user and the vault secret is the password.

Run a local Postgres for this example:

```sh
docker run --rm --name arroba-postgres-example -e POSTGRES_PASSWORD=arroba -p 5432:5432 postgres:16-alpine
```

In another shell, create sample data:

```sh
docker exec arroba-postgres-example psql -U postgres -d postgres -c "create table if not exists users (id int primary key, email text); insert into users values (1, 'user@example.com') on conflict (id) do update set email = excluded.email;"
```

Store the password, register the credential, and register the connector:

```text
/credential set postgres-local-password
/credential register examples/connectors/postgres/credential.yaml
/connector register examples/connectors/postgres/connector.yaml
```

Test and grant:

```text
/connector test postgres_example find_user --credential postgres-local --allow read --input '{"id":1}'
/connector grant <agent> postgres_example --credential postgres-local --allow read
```
