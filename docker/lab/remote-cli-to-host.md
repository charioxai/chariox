# Remote CLI From A Lab Container To The Host Kernel

This drill proves that a CLI running inside a Docker worker can connect through the relay and operate the host kernel as a remote CLI.

## Start The Lab

```sh
docker compose -f docker/lab/docker-compose.yml up -d relay worker-a worker-b
```

## Connect The Host Kernel To The Lab Relay

Open the host Chariox CLI and configure the relay:

```text
/relay use ws://127.0.0.1:43150 local-lab
```

Then get the host daemon id:

```text
/relay status
```

Use the `daemon=<id>` value in the next step.

## Start A Remote CLI Inside The Container

```sh
docker exec -it chariox-worker-a chariox \
  --relay-url ws://relay:43150 \
  --relay-token local-lab \
  --target-daemon-id <host-daemon-id>
```

Expected behavior:

- The CLI is rendered inside the container terminal.
- Session creation, session attach, workflow outline, workflow creation, prompt submission, and event streaming are served by the host kernel.
- The relay only forwards encrypted payloads and routing metadata.

## Workflow Drill

From the container-hosted remote CLI:

```text
/session new docker-remote-cli
/workflow new docker-relay-workflow
/workflow node add agent remote-check
/workflow endpoint add docker-relay-workflow run remote-check
/workflow run docker-relay-workflow run hello-from-docker-cli
```

Use the normal workflow drill commands if the exact workflow grammar has changed; the important property is that commands are entered in the container CLI and state changes appear in the host kernel.

## Remote Machine Approval

From the host CLI, after `worker-a` and `worker-b` register through the relay:

```text
/machine list
/machine approve chariox-worker-a
/machine rename chariox-worker-a docker-a
/machine kernels docker-a
```

Once a provider is installed and logged in inside the worker, it should appear in the host provider list as machine-qualified availability, for example `Codex (docker-a)`.
