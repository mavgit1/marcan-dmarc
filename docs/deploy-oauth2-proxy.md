# Caddy with OAuth2-Proxy

If you already have an OIDC identity provider (Entra ID, Okta, Google, Keycloak, ...), [OAuth2-Proxy](https://oauth2-proxy.github.io/oauth2-proxy/) sits between Caddy and Marcan: every request goes through your provider's sign-in, with whatever it enforces (two-factor authentication, passkeys, conditional access).

The files are in `deploy/oauth2-proxy/` of the repository. What the image takes and where the data goes is on [Deployment](./deployment#image-and-options).

## What you need

- A server with Docker and Compose v2 (the `docker compose` command), ports 80 and 443 open.
- A DNS name pointing at the server, such as `dmarc.example.com`, resolving before the first start: Caddy asks for the certificate right away.
- An OIDC client at your provider, with `https://dmarc.example.com/oauth2/callback` (your host name) as its redirect URI. Note its issuer URL, client id and client secret. The provider must put the user's e-mail address in the ID token: the allowed e-mail domains are checked against it.

## The files

::: code-group

<<< ../deploy/oauth2-proxy/docker-compose.yml [compose.yaml]

<<< ../deploy/oauth2-proxy/Caddyfile{txt} [Caddyfile]

<<< ../deploy/oauth2-proxy/.env.example{sh} [.env]

:::

## 1. Fetch the files

Download the three files of the release into a new directory on the server, and copy `.env.example` to `.env`:

```bash
mkdir marcan && cd marcan
base=https://raw.githubusercontent.com/mavgit1/marcan-dmarc/v0.3.0/deploy/oauth2-proxy
curl -fsSLO "$base/docker-compose.yml"
curl -fsSLO "$base/Caddyfile"
curl -fsSLO "$base/.env.example"
cp .env.example .env
```

Use the tag of the release you install in place of `v0.3.0`. A clone of the repository has the same files in `deploy/oauth2-proxy/`; run from there, the volume is `oauth2-proxy_marcan_data` instead of `marcan_marcan_data` (see [The data volume](./deployment#the-data-volume)).

## 2. Fill in `.env`

| Variable | What |
| :--- | :--- |
| `MARCAN_DOMAIN` | The host name, such as `dmarc.example.com` |
| `OIDC_ISSUER_URL` | Your provider's issuer, for Entra ID `https://login.microsoftonline.com/<tenant-id>/v2.0` |
| `OIDC_CLIENT_ID`, `OIDC_CLIENT_SECRET` | The client you registered |
| `ALLOWED_EMAIL_DOMAINS` | Who may sign in, such as `example.com`; several comma-separated |
| `COOKIE_SECRET` | `openssl rand -base64 32` |

Compose refuses to start while one of them is missing.

## 3. The logins

There are none to create here: whoever your provider signs in with an e-mail address in `ALLOWED_EMAIL_DOMAINS` gets in. Every signed-in user can change every setting, so allow only your own e-mail domain(s), never `*` with a public provider such as Google. To narrow it further, restrict who may use the client at your provider (for Entra ID: assignment required, then assign users or a group).

## 4. Start it

```bash
docker compose up -d
```

## 5. Check it

```bash
curl https://dmarc.example.com/healthz   # ok
docker compose ps                        # marcan-app healthy, marcan-oauth2-proxy and marcan-caddy up
```

Open `https://dmarc.example.com`: you are sent to your provider to sign in, then back to Marcan. Follow the setup (see [First start](./getting-started#first-start)). An error at the provider about the redirect URI means it does not match `https://<MARCAN_DOMAIN>/oauth2/callback` exactly; a "403 Forbidden" after signing in means your address is not in `ALLOWED_EMAIL_DOMAINS`. `docker compose logs oauth2-proxy` shows both.

## Upgrade

Read the [release notes](./releases), Read the [release notes](./releases) first; a [database backup](./backup#the-database-file) is only needed to be able to go back a version. The steps, and how to pin a version, are in [Upgrade](./upgrade). The OAuth2-Proxy image is pinned in `docker-compose.yml`; a Marcan release that moves it says so in its [release notes](./releases).
