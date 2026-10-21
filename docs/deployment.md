# Deploying Marcan

A permanent Marcan is always two parts: Marcan itself, and a reverse proxy in front of it that handles HTTPS and the login. Marcan has no accounts of its own, so you choose how strong the login is, from one shared password to single sign-on with two-factor authentication (see [Security model](./security)).

The proxy must be the only way in. In the stacks below Marcan's port is never published: Docker makes it reachable only from the proxy on the same Compose network, not from the server's network.

## Choose a setup

`deploy/` in the repository holds one Docker Compose stack per folder. Each has its own page with every file, what goes into `.env`, how the login is created and how to check the first start:

| Setup | Folder | When to use it |
| :--- | :--- | :--- |
| [Caddy with basic auth](./deploy-caddy) | `deploy/caddy/` | The default: one shared login, nothing else to run |
| [Caddy with Authelia](./deploy-authelia) | `deploy/authelia/` | Several users, a login page, two-factor authentication |
| [Caddy with OAuth2-Proxy](./deploy-oauth2-proxy) | `deploy/oauth2-proxy/` | You already have an OIDC identity provider (Entra ID, Okta, Google, Keycloak) |
| [Your own proxy](#your-own-reverse-proxy) | | Nginx, Traefik, an ingress controller, a tunnel, ... |
| [Local trial](#local-trial) | | Looking at it on your own machine |

Every stack needs a server with Docker and Compose v2 (the `docker compose` command; the old separate `docker-compose` does not read these files), ports 80 and 443 open, and DNS names that point at the server before the first start.

## Image and options

The image is `ghcr.io/mavgit1/marcan`, for `linux/amd64` and `linux/arm64`. Each release is tagged `X.Y.Z`, `X.Y` and `latest`. Pin a version for anything you rely on, and upgrade on purpose (see [Upgrade](./upgrade)). The Compose stacks read the version from `MARCAN_VERSION` in `.env`.

The image runs as an unprivileged user (uid/gid 10001) and keeps its database in `/data`. It has a built-in health check against `/healthz`.

**Marcan itself needs no configuration file.** Everything you set up (mailbox, domains, lists) lives in its database and is changed in the UI. The `.env` of a stack configures the proxy and the login, and the Marcan version.

The server takes three options, as flags or environment variables:

| Flag | Variable | Description | Default |
| :--- | :--- | :--- | :--- |
| `--bind` | `BIND_ADDR` | Address and port to listen on | `127.0.0.1:8080` (image: `0.0.0.0:8080`) |
| `--data-dir` | `DATA_DIR` | Directory holding `marcan.db` | `.` (image: `/data`) |
| `--allowed-hosts` | `ALLOWED_HOSTS` | Host names requests may be addressed to besides the bind address and `localhost`, comma-separated | none |

When Marcan listens on one address, such as the default `127.0.0.1:8080`, it answers only requests for that address, `localhost` and the names in `--allowed-hosts`, so a web page cannot reach it by pointing its own name at your machine (see [Security](./security#host-names-and-frames)). If you run the binary without Docker behind a proxy on the same server, add the name the proxy forwards, for example `--allowed-hosts dmarc.example.com`: Caddy and nginx pass the original host name on. The image listens on `0.0.0.0:8080` and answers any host there, so the Compose stacks need nothing.

Logging follows `RUST_LOG` (default `info`). The server shuts down cleanly on SIGTERM and Ctrl-C. Everything else (the mailbox, domains, lists, DNS resolvers) is set in the UI under **Settings** and applies without a restart.

Every stack serves `/healthz` without a login (it answers `ok`, nothing else) for uptime monitors. Everything else, including `/api/system/health`, needs a login.

### The data volume

The stacks keep `/data` in a named Docker volume, `marcan_data` in the Compose file, which Docker creates writable for the container's user. Docker puts the Compose project's name in front, by default the directory's name: in a directory `marcan`, the volume is `marcan_marcan_data`, and `docker volume` commands need that full name (`docker volume ls` lists it). The stack pages download the files into a directory `marcan` for that reason; run from a clone, the project is the stack's folder name (`caddy`, `authelia`, `oauth2-proxy`). To keep a volume after moving or renaming the directory, set `COMPOSE_PROJECT_NAME` in `.env` to the old name.

A directory on the host (a bind mount, `./data:/data`) works too, but every database write then goes through the host's file sharing: on Docker Desktop and on some NAS filesystems that makes the first import of a large mailbox many times slower. To use one anyway, create it writable for the container's user first (`mkdir -p data && sudo chown 10001:10001 data`) and replace `marcan_data:/data` with `./data:/data`.

## Your own reverse proxy

Use the image directly and put whatever you like in front of it, as long as it authenticates every request except, optionally, `/healthz`:

```yaml
services:
  marcan:
    image: ghcr.io/mavgit1/marcan:<version>
    restart: unless-stopped
    volumes:
      - marcan_data:/data
    # No "ports:". Join the network your proxy is on and proxy to marcan:8080.

volumes:
  marcan_data:
```

Marcan reads no authentication headers: it does not know who is logged in, and it does not need to. Nothing needs to be forwarded except the request itself. If you publish the port anyway (`ports:`), bind it to an address that only your proxy can reach, for example `127.0.0.1:8080:8080`.

Docker creates the named volume writable for uid/gid 10001. A bind mount instead (`./data:/data`) must be made writable for that user first, and is slower (see [The data volume](#the-data-volume)).

## Local trial

To look at Marcan on your own machine without a proxy:

```bash
docker run --rm -p 127.0.0.1:8080:8080 -v marcan-data:/data ghcr.io/mavgit1/marcan:latest
```

Then open <http://localhost:8080> and follow the setup. The `127.0.0.1:` in `-p` is what keeps the port reachable from your own machine only. Without it, Docker publishes the port on all interfaces and anyone on your network could open Marcan, with no login. Do not leave this running on a server, and do not remove the `127.0.0.1:` part: to reach Marcan from other machines, use one of the [stacks above](#choose-a-setup), with the reverse proxy in front. Stop it with Ctrl-C, and remove the data with `docker volume rm marcan-data` when you are done.

Add `-e ALLOWED_HOSTS=localhost` to the command to have the trial answer only requests for `localhost` and `127.0.0.1`, so that a web page cannot reach it by pointing its own name at your machine.

This is for trying it out, not for keeping: reports keep arriving every day, so Marcan is meant to run all the time.

## Network access

Marcan needs outbound access only:

- IMAP over TLS (port 993) or STARTTLS (port 143) to your mailbox host,
- DNS (port 53) to its resolvers: Cloudflare `1.1.1.1` and Google `8.8.8.8` by default, or your own under **Settings → DNS checks → Resolvers**,
- HTTPS to `mta-sts.<domain>` for each domain that publishes an MTA-STS record,
- with sending on (**Settings → Sending**), SMTP submission to your provider's SMTP server, usually port 465 with TLS or 587 with STARTTLS (never port 25),
- unless the version check is switched off (**Settings → License**, on by default), HTTPS to `marcan.kaitiaki.tech` (GitHub Pages) once a day (see [Upgrade](./upgrade#checking-for-new-versions)).

Nothing connects to Marcan except your proxy.
