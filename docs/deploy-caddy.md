# Caddy with basic auth

The default setup: [Caddy](https://caddyserver.com/) in front of Marcan, with one shared user name and password. Caddy listens on ports 80 and 443, gets and renews HTTPS certificates on its own, and asks for the login. Marcan's port is not published: it is reachable only on the internal Compose network.

The files are in `deploy/caddy/` of the repository. What the image takes and where the data goes is on [Deployment](./deployment#image-and-options).

## What you need

- A server with Docker and Compose v2 (the `docker compose` command; the old separate `docker-compose` does not read these files).
- Ports 80 and 443 open.
- A DNS name pointing at the server, such as `dmarc.example.com`. It must resolve to the server before the first start: Caddy asks for the certificate right away, and while the name does not lead to it the request fails and Caddy waits longer before each new try.

## The files

::: code-group

<<< ../deploy/caddy/docker-compose.yml [compose.yaml]

<<< ../deploy/caddy/Caddyfile{txt} [Caddyfile]

<<< ../deploy/caddy/.env.example{sh} [.env]

:::

## 1. Fetch the files

Download the three files of the release into a new directory on the server, and copy `.env.example` to `.env`:

```bash
mkdir marcan && cd marcan
base=https://raw.githubusercontent.com/mavgit1/marcan-dmarc/v0.3.0/deploy/caddy
curl -fsSLO "$base/docker-compose.yml"
curl -fsSLO "$base/Caddyfile"
curl -fsSLO "$base/.env.example"
cp .env.example .env
```

Use the tag of the release you install in place of `v0.3.0`. A clone of the repository has the same files in `deploy/caddy/`. The database goes into the named volume `marcan_marcan_data` (the directory's name, then `marcan_data`), which Compose creates on the first start; see [The data volume](./deployment#the-data-volume).

## 2. Fill in `.env`

| Variable | What |
| :--- | :--- |
| `MARCAN_DOMAIN` | The host name, such as `dmarc.example.com` |
| `MARCAN_USER` | The login name |
| `MARCAN_PASSWORD_HASH` | The password's bcrypt hash, below |

Compose refuses to start while one of them is missing.

## 3. Create the login

Generate the password hash and put it into `.env`:

```bash
docker run --rm -it caddy:2-alpine caddy hash-password
```

Keep the hash in single quotes in `.env`: it contains `$` characters.

## 4. Start it

```bash
docker compose up -d
```

## 5. Check it

```bash
curl https://dmarc.example.com/healthz   # ok
docker compose ps                        # marcan-app healthy, marcan-caddy up
```

Open `https://dmarc.example.com`, sign in with the user and the password you hashed, and follow the setup (see [First start](./getting-started#first-start)). If the certificate is missing, `docker compose logs caddy` says why; usually the DNS name does not lead to the server yet.

Basic auth means one shared login for everyone, and the browser keeps it until it is closed. Use [Authelia](./deploy-authelia) if you need separate users or two-factor authentication.

## Upgrade

Read the [release notes](./releases), Read the [release notes](./releases) first; a [database backup](./backup#the-database-file) is only needed to be able to go back a version. The steps, and how to pin a version, are in [Upgrade](./upgrade).

Before 0.3.0 these files were in `deploy/` itself. If you ran the stack from `deploy/` in a clone, its volume is `deploy_marcan_data`: move your `.env` to `deploy/caddy/` and add `COMPOSE_PROJECT_NAME=deploy` to it, so Compose finds the volume again. With a bind mount instead, move `data/` along. A stack in a directory of its own, as above, is not affected.
