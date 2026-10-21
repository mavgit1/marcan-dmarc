# Development

Marcan is a Rust workspace with a SvelteKit UI that is built into the binary.

## Layout

| Path | What |
| :--- | :--- |
| `crates/dmarc-server` | The binary: the axum API under `/api`, background jobs, the embedded UI |
| `crates/dmarc-store` | SQLite storage (sqlx), queries by area returning plain rows |
| `crates/dmarc-parser` | Aggregate report XML (plain, gzip, zip) and RFC 7489 alignment |
| `crates/dmarc-imap` | The IMAP fetch pass and polling loop. Knows no database: it talks to a `Backend` trait the server implements over the store; `MemoryBackend` and `fake_server.rs` for tests |
| `crates/dmarc-dns` | SPF, DMARC, DKIM, MTA-STS, TLS-RPT and BIMI lookups and record parsers; reports findings in `dmarc-assess`'s vocabulary |
| `crates/dmarc-assess` | Every verdict and threshold the UI shows, the per-record facts and the DNS score included. Pure: no I/O, no clock |
| `crates/dmarc-identity` | Sender identity from reverse DNS and SPF evidence, the bundled name map |
| `crates/dmarc-domain` | The one place for hostname normalization, the organizational domain (Public Suffix List) and the RFC 7489 §7.1 authorization rules |
| `migrations/` | sqlx migrations, embedded into the binary |
| `web/` | The UI: SvelteKit static SPA, Svelte 5, Tailwind 4 |
| `docs/` | This site (VitePress) |
| `deploy/` | Example Compose stacks: `caddy/`, `authelia/`, `oauth2-proxy/` |

Judgements live in Rust, in `dmarc-assess`. The one exception is the record outcome (passed, delivered, forwarded, blocked): the facts are computed at ingest into columns on `records`, and the four-way rule over them is a SQL expression in `dmarc-store` (`outcome.rs`) that every count, filter and export reads. The UI only words and colours what the server decided; numbers it shows in help text come from `GET /api/meta`.

## Requirements

- Rust: the version in `rust-toolchain.toml` (rustup installs it automatically)
- Node.js 24 and pnpm 12

## Run it

Run the API and the Vite dev server side by side. Vite proxies `/api` to the API.

```bash
# Terminal 1: API on :8080, data in .dev/
cargo run -p dmarc-server -- --data-dir .dev
```

```bash
# Terminal 2: UI on http://localhost:5173
cd web && pnpm install && pnpm dev
```

A debug build runs without the built UI (`web/build/`); a release build embeds it and fails without it.

## Demo data

To try the UI without a mailbox, fill a separate database with 30 days of made-up reports for the example domains, then run a server on it:

```bash
cargo run -p dmarc-store --example demo-data -- .demo
cargo run -p dmarc-server -- --data-dir .demo --bind 127.0.0.1:8090
```

The docs' screenshots are taken from this data, so they never show real domains.

## Build a release binary

```bash
cd web && pnpm install && pnpm build && cd ..
cargo build --release -p dmarc-server   # target/release/dmarc-server
```

Or the image: `docker build -t marcan .`

## Checks

```bash
scripts/check.sh
```

It builds the UI first (the server embeds `web/build`, and its tests serve it), then runs `cargo fmt --check`, a dependency audit (`cargo audit` for known advisories, unmaintained and yanked crates; `cargo deny` for the licenses, crates and sources `deny.toml` allows; both installed with `cargo install --locked` on first use, and an advisory that cannot be fixed yet is ignored in `deny.toml` with its reason), the Rust tests, clippy with warnings as errors (the default lints plus the curated list under `[workspace.lints.clippy]` in `Cargo.toml`), the UI type check, ESLint (`web/eslint.config.js`, zero problems), Prettier (`pnpm format:check`; `pnpm format` rewrites) and unit tests (Vitest, `web/src/**/*.test.ts`), and a docs build that fails on dead links. It also fails when the generated TypeScript types are out of date. It installs the dependencies of `web/` and `docs/` from their lockfiles when `node_modules` is missing. Run it before every push; to do that automatically:

```bash
git config core.hooksPath .githooks
```

## API types

The UI's API types in `web/src/lib/types/` are generated from the Rust types by [ts-rs](https://github.com/Aleph-Alpha/ts-rs) whenever `cargo test` runs. Never edit them by hand: change the Rust type, run the tests, and commit both.

## Database changes

Migrations in `migrations/` are checksummed. Never edit one that has been released: existing databases would refuse to open. Add a new `migrations/NNNN_description.sql` instead.

## Sender name data

`crates/dmarc-identity/data/` holds parsedmarc's reverse-DNS name map (CC BY-SA 4.0, with network data from [IPinfo](https://ipinfo.io/)) and Public Suffix List overrides (Apache 2.0), unmodified. `scripts/update-sender-names.sh` refreshes them and summarizes what changed.

## Docs

```bash
cd docs && pnpm install && pnpm dev
```

Pages are Markdown files in `docs/`. The site is published to GitHub Pages with each release.
