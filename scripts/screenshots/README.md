# Docs screenshots

1. Data: `cargo run -p dmarc-store --example realistic-demo -- /tmp/shots-data --seed 7`, and an empty `/tmp/shots-fresh` for the setup.
2. Servers (after `pnpm --dir web build`): `cargo run -p dmarc-server -- --data-dir /tmp/shots-data --bind 127.0.0.1:8711`, and the same on `/tmp/shots-fresh` at `127.0.0.1:8712`.
3. Shoot: `pnpm install` here (installs Chromium), then `node shoot.mjs http://127.0.0.1:8711 ../../docs/public/screenshots http://127.0.0.1:8712` (`--only name,name` for a few).
4. Pages, each `<name>-light.png` and `-dark.png` at 1440x900, 2x: dashboard, domains, domain and failures (a p=none domain with issues), organizations, senders, sender, alerts, dns (inspector), sending, client-report (preview), setup (Mailbox step).
