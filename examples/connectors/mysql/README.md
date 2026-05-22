# MySQL Connector Example

The `mysql` adapter runs SQL with positional `?` parameters and returns rows as JSON.

Credentials can use `basic` injection, where `username` is the database user and the vault secret is the password.

Run a local MySQL for this example:

```sh
docker run --rm --name arroba-mysql-example -e MYSQL_ROOT_PASSWORD=arroba -e MYSQL_DATABASE=arroba -p 3306:3306 mysql:8
```

In another shell, create sample data:

```sh
docker exec arroba-mysql-example mysql -uroot -parroba arroba -e "create table if not exists users (id int primary key, email varchar(255)); insert into users values (1, 'user@example.com') on duplicate key update email = values(email);"
```

Store the password, register the credential, and register the connector:

```text
/credential set mysql-local-password
/credential register examples/connectors/mysql/credential.yaml
/connector register examples/connectors/mysql/connector.yaml
```

Test and grant:

```text
/connector test mysql_example find_user --credential mysql-local --allow read --input '{"id":1}'
/connector grant <agent> mysql_example --credential mysql-local --allow read
```
