# Deploying Marcan

A permanent Marcan is always two parts: Marcan itself, and a reverse proxy in front of it that handles HTTPS and the login. Marcan has no accounts of its own, so you choose how strong the login is, from one shared password to single sign-on with two-factor authentication (see [Security model](./security)).

The proxy must be the only way in. In the examples below Marcan's port is never published: Docker makes it reachable only from the proxy on the same Compose network, not from the server's network.

The image runs as an unprivileged user (uid/gid 10001) and keeps its database in `/data`. It has a built-in health check against `/healthz`.

The stacks below keep `/data` in a named Docker volume, `marcan_data` in the Compose file, which Docker creates writable for that user. Docker puts the Compose project's name in front, by default the directory's name: in a directory `marcan`, the volume is `marcan_marcan_data`, and `docker volume` commands need that full name (`docker volume ls` lists it). A directory on the host (a bind mount, `./data:/data`) works too, but every database write then goes through the host's file sharing: on Docker Desktop and on some NAS filesystems that makes the first import of a large mailbox many times slower. To use one anyway, create it writable for the container's user first (`mkdir -p data && sudo chown 10001:10001 data`) and replace `marcan_data:/data` with `./data:/data`.

| Setup | When to use it |
| :--- | :--- |
| [Caddy with basic auth](#default-caddy-with-basic-auth) | The default: one shared login, nothing else to run |
| [Caddy with Authelia](#sso-with-authelia) | Several users, a login page, two-factor authentication |
| [OAuth2-Proxy](#alternative-oauth2-proxy) | You already have an OIDC identity provider (Entra ID, Okta, Google, Keycloak) |
| [Your own proxy](#your-own-reverse-proxy) | Nginx, Traefik, an ingress controller, a tunnel, ... |
| [Local trial](#local-trial) | Looking at it on your own machine |

## Image and options

The image is `ghcr.io/mavgit1/marcan`, for `linux/amd64` and `linux/arm64`. Each release is tagged `X.Y.Z`, `X.Y` and `latest`. Pin a version for anything you rely on, and upgrade on purpose (see [Backup & upgrades](./backup)). The Compose examples read the version from `MARCAN_VERSION` in `.env`.

The server takes three options, as flags or environment variables:

| Flag | Variable | Description | Default |
| :--- | :--- | :--- | :--- |
| `--bind` | `BIND_ADDR` | Address and port to listen on | `127.0.0.1:8080` (image: `0.0.0.0:8080`) |
| `--data-dir` | `DATA_DIR` | Directory holding `marcan.db` | `.` (image: `/data`) |
| `--allowed-hosts` | `ALLOWED_HOSTS` | Host names requests may be addressed to besides the bind address and `localhost`, comma-separated | none |

When Marcan listens on one address, such as the default `127.0.0.1:8080`, it answers only requests for that address, `localhost` and the names in `--allowed-hosts`, so a web page cannot reach it by pointing its own name at your machine (see [Security](./security#host-names-and-frames)). If you run the binary without Docker behind a proxy on the same server, add the name the proxy forwards, for example `--allowed-hosts dmarc.example.com`: Caddy and nginx pass the original host name on. The image listens on `0.0.0.0:8080` and answers any host there, so the Compose stacks below need nothing.

Logging follows `RUST_LOG` (default `info`). The server shuts down cleanly on SIGTERM and Ctrl-C. Everything else (the mailbox, domains, lists, DNS resolvers) is set in the UI under **Settings** and applies without a restart.

Every example serves `/healthz` without a login (it answers `ok`, nothing else) for uptime monitors. Everything else, including `/api/system/health`, needs a login.

## Default: Caddy with basic auth

`deploy/` holds a Docker Compose stack with [Caddy](https://caddyserver.com/) in front of Marcan. Caddy listens on ports 80 and 443, gets and renews HTTPS certificates on its own, and asks for a user name and password. Marcan's port is not published: it is reachable only on the internal Compose network.

You need a server with Docker and Compose v2 (the `docker compose` command; the old separate `docker-compose` does not read these files), ports 80 and 443 open, and a DNS name pointing at it. The name must resolve to the server before the first start: Caddy asks for the certificate right away, and while the name does not lead to it the request fails and Caddy waits longer before each new try.

**Marcan itself needs no configuration file.** Everything you set up (mailbox, domains, lists) lives in its database and is changed in the UI. The `.env` below only configures Caddy: the host name and the login.

::: code-group

<<< ../deploy/docker-compose.yml [compose.yaml]

<<< ../deploy/Caddyfile{txt} [Caddyfile]

<<< ../deploy/.env.example{sh} [.env]

:::

1. Download the three files of the release into a new directory on the server, and copy `.env.example` to `.env`:

   ```bash
   mkdir marcan && cd marcan
   base=https://raw.githubusercontent.com/mavgit1/marcan-dmarc/v0.2.0/deploy
   curl -fsSLO "$base/docker-compose.yml"
   curl -fsSLO "$base/Caddyfile"
   curl -fsSLO "$base/.env.example"
   cp .env.example .env
   ```

   Use the tag of the release you install in place of `v0.2.0`. A clone of the repository has the same files in `deploy/`. The database goes into the named volume `marcan_marcan_data` (the directory's name, then `marcan_data`), which Compose creates on the first start.
2. Generate the password hash and put it into `.env` with the host name and user name:

   ```bash
   docker run --rm -it caddy:2-alpine caddy hash-password
   ```

   Keep the hash in single quotes in `.env`: it contains `$` characters. Compose refuses to start while one of the three values is missing.

3. Start it and open the site:

   ```bash
   docker compose up -d
   curl https://dmarc.example.com/healthz   # ok
   ```

Sign in with the user and the password you hashed, and follow the setup.

Basic auth means one shared login for everyone, and the browser keeps it until it is closed. Use Authelia below if you need separate users or two-factor authentication.

## SSO with Authelia

`deploy/authelia/` runs [Authelia](https://www.authelia.com/) next to Caddy: a login page with its own user file and two-factor authentication (TOTP or WebAuthn), no external identity provider needed. Marcan is only reachable after Authelia has accepted the session.

You need two DNS names pointing at the server, both resolving before the first start: one for Marcan (`dmarc.example.com`) and one for the login portal (`auth.example.com`). They must share a parent domain (`example.com`), because the session cookie is set for it.

This stack has more files than the default one, so take it from a clone of the repository at the release tag:

```bash
git clone --depth 1 --branch v0.2.0 https://github.com/mavgit1/marcan-dmarc.git
cd marcan-dmarc/deploy/authelia
cp .env.example .env
cp config/users.yml.example config/users.yml
```

1. In `.env`, set `MARCAN_DOMAIN`, `AUTH_DOMAIN`, `COOKIE_DOMAIN` and the three secrets (`openssl rand -hex 32` for each).
2. Generate a password hash and paste the `Digest:` value into `config/users.yml`, where you can also add more users:

   ```bash
   docker run --rm -it authelia/authelia:4.39 authelia crypto hash generate argon2
   ```

3. `docker compose up -d`, then open `https://dmarc.example.com`. You are sent to the portal to sign in.

`config/configuration.yml` requires two-factor (`two_factor`); change the rule to `one_factor` to ask for the password only. Authelia does not send mail in this setup: the one-time code you need when registering a 2FA device is written to `config/notification.txt` (`cat` it on the server). Keep `config/users.yml` and `config/db.sqlite3` out of version control and back them up with the rest.

Authelia is configured through its own configuration file; see its documentation for e-mail delivery, LDAP, OIDC and access rules per group.

## Alternative: OAuth2-Proxy

If you already have an OIDC identity provider, `deploy/oauth2-proxy/` puts [OAuth2-Proxy](https://oauth2-proxy.github.io/oauth2-proxy/) between Caddy and Marcan. Take the folder from a clone as for Authelia (`cd marcan-dmarc/deploy/oauth2-proxy`), copy `.env.example` to `.env` there and fill in the issuer URL, client id and secret, the allowed e-mail domain(s) and a cookie secret (`openssl rand -base64 32`); register `https://<MARCAN_DOMAIN>/oauth2/callback` as the redirect URI at your provider. Then `docker compose up -d`.

Every signed-in user can change every setting, so allow only your own e-mail domain(s) in `ALLOWED_EMAIL_DOMAINS`, never `*` with a public provider such as Google.

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

Docker creates the named volume writable for uid/gid 10001. A bind mount instead (`./data:/data`) must be made writable for that user first, and is slower (see above).

## Local trial

To look at Marcan on your own machine without a proxy:

```bash
docker run --rm -p 127.0.0.1:8080:8080 -v marcan-data:/data ghcr.io/mavgit1/marcan:latest
```

Then open <http://localhost:8080>. The `127.0.0.1:` in `-p` is what keeps the port reachable from your own machine only. Without it, Docker publishes the port on all interfaces and anyone on your network could open Marcan, with no login. Do not leave this running on a server, and do not remove the `127.0.0.1:` part: to reach Marcan from other machines, use one of the stacks in `deploy/` described above, with the reverse proxy in front. Remove the container and the data with `docker volume rm marcan-data` when you are done.

Add `-e ALLOWED_HOSTS=localhost` to the command to have the trial answer only requests for `localhost` and `127.0.0.1`, so that a web page cannot reach it by pointing its own name at your machine.

## Network access

Marcan needs outbound access only:

- IMAP over TLS (port 993) or STARTTLS (port 143) to your mailbox host,
- DNS (port 53) to its resolvers: Cloudflare `1.1.1.1` and Google `8.8.8.8` by default, or your own under **Settings → DNS checks → Resolvers**,
- HTTPS to `mta-sts.<domain>` for each domain that publishes an MTA-STS record,
- with sending on (**Settings → Sending**), SMTP submission to your provider's SMTP server, usually port 465 with TLS or 587 with STARTTLS (never port 25).

Nothing connects to Marcan except your proxy.
