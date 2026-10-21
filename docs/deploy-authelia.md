# Caddy with Authelia

[Authelia](https://www.authelia.com/) runs next to Caddy: a login page with its own user file and two-factor authentication (TOTP or WebAuthn), no external identity provider needed. Marcan is only reachable after Authelia has accepted the session.

The files are in `deploy/authelia/` of the repository. What the image takes and where the data goes is on [Deployment](./deployment#image-and-options).

## What you need

- A server with Docker and Compose v2 (the `docker compose` command), ports 80 and 443 open.
- Two DNS names pointing at the server, both resolving before the first start: one for Marcan (`dmarc.example.com`) and one for the login portal (`auth.example.com`). They must share a parent domain (`example.com`), because the session cookie is set for it. Caddy asks for both certificates right away.

## The files

::: code-group

<<< ../deploy/authelia/docker-compose.yml [compose.yaml]

<<< ../deploy/authelia/Caddyfile{txt} [Caddyfile]

<<< ../deploy/authelia/.env.example{sh} [.env]

<<< ../deploy/authelia/config/configuration.yml [config/configuration.yml]

<<< ../deploy/authelia/config/users.yml.example{yaml} [config/users.yml]

:::

## 1. Fetch the files

Download the five files of the release into a new directory on the server:

```bash
mkdir -p marcan/config && cd marcan
base=https://raw.githubusercontent.com/mavgit1/marcan-dmarc/v0.3.0/deploy/authelia
curl -fsSLO "$base/docker-compose.yml"
curl -fsSLO "$base/Caddyfile"
curl -fsSLO "$base/.env.example"
curl -fsSL "$base/config/configuration.yml" -o config/configuration.yml
curl -fsSL "$base/config/users.yml.example" -o config/users.yml.example
cp .env.example .env
cp config/users.yml.example config/users.yml
```

Use the tag of the release you install in place of `v0.3.0`. A clone of the repository has the same files in `deploy/authelia/`; run from there, the volume is `authelia_marcan_data` instead of `marcan_marcan_data` (see [The data volume](./deployment#the-data-volume)).

## 2. Fill in `.env`

| Variable | What |
| :--- | :--- |
| `MARCAN_DOMAIN` | Marcan's host name, such as `dmarc.example.com` |
| `AUTH_DOMAIN` | The login portal's host name, such as `auth.example.com` |
| `COOKIE_DOMAIN` | A parent domain of both, such as `example.com` |
| `AUTHELIA_JWT_SECRET`, `AUTHELIA_SESSION_SECRET`, `AUTHELIA_STORAGE_ENCRYPTION_KEY` | Three independent secrets: `openssl rand -hex 32` for each |
| `MARCAN_VERSION` | Optional: the Marcan version, the release's own by default |

Compose refuses to start while one of them is missing. Changing the encryption key later makes the stored 2FA registrations unreadable.

`config/configuration.yml` reads the host names from the environment, so there is normally nothing to edit in it.

## 3. Create the logins

Generate a password hash and paste the `Digest:` value into `config/users.yml` (keep the quotes):

```bash
docker run --rm -it authelia/authelia:4.39 authelia crypto hash generate argon2
```

Add one block under `users:` for each person, with a hash of their own. `config/configuration.yml` requires two-factor (`two_factor`); change the rule to `one_factor` to ask for the password only.

## 4. Start it

```bash
docker compose up -d
```

## 5. Check it

```bash
curl https://dmarc.example.com/healthz   # ok
docker compose ps                        # marcan-app healthy, marcan-authelia and marcan-caddy up
```

Open `https://dmarc.example.com`: you are sent to the portal at `auth.example.com` to sign in, then back to Marcan. Follow the setup (see [First start](./getting-started#first-start)).

Authelia does not send mail in this setup: the one-time code you need when registering a 2FA device is written to `config/notification.txt` (`cat` it on the server). If the portal does not come up, `docker compose logs authelia` names the setting it rejects.

Keep `config/users.yml` and `config/db.sqlite3` out of version control and back them up with the rest.

Authelia is configured through its own configuration file; see its documentation for e-mail delivery, LDAP, OIDC and access rules per group.

## Upgrade

Set the new version in `.env`, then `docker compose pull && docker compose up -d`. Take a [backup](./backup) first; the steps are in [Upgrade](./upgrade). The Authelia image is pinned in `docker-compose.yml` (`authelia/authelia:4.39`); a Marcan release that moves it says so in its [release notes](./releases).
