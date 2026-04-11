# Arroba Docker Remote-Machine Lab

This lab image models Docker containers as ordinary Arroba machines. The image includes all Arroba apps: the CLI, daemon/kernel, and relay. It does not install provider CLIs or manage provider login for the user.

The main purpose is to test remote-machine behavior locally and to let users keep multiple provider accounts active at the same time by isolating each account in a persistent container home directory.

## Start The Lab

```sh
docker compose -f docker/lab/docker-compose.yml up -d relay worker-a worker-b
```

The compose file starts a local relay and two worker kernels. Each worker has a persistent `/home/arroba` volume, so Arroba machine identity, provider installs, and provider credentials survive container restarts.

The relay listens on `ws://127.0.0.1:43150` with the default lab token `local-lab`. Override it with `ARROBA_RELAY_TOKEN` when starting compose:

```sh
ARROBA_RELAY_TOKEN=change-me docker compose -f docker/lab/docker-compose.yml up -d relay worker-a worker-b
```

From the host Arroba CLI, point the home kernel at the lab relay:

```text
/relay use ws://127.0.0.1:43150 local-lab
/machine list
```

## Use Separate Provider Accounts

Enter each worker and install/log into providers manually:

```sh
docker exec -it arroba-worker-a zsh
# install provider CLIs using their Linux instructions
# run provider-native login, for example: codex login
```

```sh
docker exec -it arroba-worker-b zsh
# log into a different account for the same provider if desired
```

Once provider login works inside a worker, the home kernel can expose it as a machine-qualified provider such as `Codex (arroba-worker-a)` and `Codex (arroba-worker-b)`.

## Included Apps

The base image includes:

- `arroba` / `arroba-cli` for the TypeScript CLI launcher
- `arroba-daemon` for the kernel/daemon
- `arroba-relay` for the self-hosted relay
- Node, pnpm, Bun, zsh, git, curl, ripgrep, jq, OpenSSH client, and basic process tools

Provider CLIs are intentionally not included. Install them inside the worker container whose provider account they should use.

## Networking

Real provider execution requires outbound internet from the container for model APIs, provider install commands, hosted relay access, and authentication flows. Docker bridge networking allows outbound internet by default. Proxy variables are passed through by the compose file:

```sh
HTTP_PROXY=http://proxy.example:8080 HTTPS_PROXY=http://proxy.example:8080 docker compose -f docker/lab/docker-compose.yml up -d
```

Normal provider runtime ports do not need host mapping when the worker kernel and provider process run in the same container. The home kernel communicates with the worker through the relay.

Login callback ports are provider-specific. The compose file publishes `39000-39049` for `worker-a` and `39050-39099` for `worker-b` as a convenience for providers that allow fixed callback ports. Providers that use device-code login or print a URL usually do not need these mappings.

## Browser Login

The base image sets `BROWSER=arroba-open-url` and places an `xdg-open` shim earlier in `PATH`. When a provider asks to open a browser through either path, the helper prints the URL so you can open it on the host.

If a provider uses an unconfigurable random localhost callback port, test and document that provider separately. The launch-provider compatibility matrix should record the tested provider version, login method, callback behavior, and whether Docker login is supported.

## Useful Commands

Build only the image:

```sh
docker build -f docker/lab/Dockerfile -t arroba-lab:latest .
```

Run a shell:

```sh
docker exec -it arroba-worker-a zsh
```

Run the CLI inside a worker:

```sh
docker exec -it arroba-worker-a arroba
```

Restart a worker:

```sh
docker compose -f docker/lab/docker-compose.yml restart worker-a
```

View logs:

```sh
docker compose -f docker/lab/docker-compose.yml logs -f relay worker-a worker-b
```

Destroy lab state, including provider credentials and machine identities:

```sh
docker compose -f docker/lab/docker-compose.yml down -v
```
