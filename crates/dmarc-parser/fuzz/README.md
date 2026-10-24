# dmarc-parser fuzz targets

Fuzzing for the attachment bytes anyone can mail to the report mailbox. A
separate crate with its own workspace, so the main build, `cargo test` and
`scripts/check.sh` never build it.

| Target | Input |
| :--- | :--- |
| `parse_report` | Raw attachment bytes through `parse_report`, once per hint (`.xml`, `.gz`, `.zip`, `.xml.gz`, none) |
| `feedback_xml` | UTF-8 text straight into the XML stage (quick-xml, serde, field checks) |

Both run the alignment checks ingest does on every report that parses
(`fuzz_targets/common.rs`).

Needs nightly and cargo-fuzz (`rustup toolchain install nightly --profile minimal`,
`cargo install --locked cargo-fuzz`); `rust-toolchain.toml` stays on stable. From
this directory:

```bash
cargo +nightly fuzz run parse_report corpus/parse_report seeds/parse_report -- \
  -dict=dmarc.dict -jobs=4 -workers=4 -max_total_time=1200 -rss_limit_mb=1024 -timeout=10 -max_len=65536
cargo +nightly fuzz run feedback_xml corpus/feedback_xml seeds/feedback_xml -- \
  -dict=dmarc.dict -jobs=4 -workers=4 -max_total_time=1200 -rss_limit_mb=1024 -timeout=10 -max_len=65536
```

The memory limit and the per-input timeout make a blow-up or a hang a finding,
like a crash. Findings land in `artifacts/<target>/`; `cargo +nightly fuzz tmin
<target> <file>` shrinks one, and the shrunk input goes into the crate's
`tests/` as a regression test. `corpus/`, `artifacts/` and `target/` are not
committed.

`seeds/` holds the reports from the crate's and the store's tests (plain, gzip
and zip, plus a zip whose report is not the first entry and a zip carrying a
gzip); `dmarc.dict` the element names. `Cargo.lock` started as a copy of the
workspace's, so the fuzzers run the same quick-xml, flate2 and zip as the
release build: copy it again after a dependency update (cargo then drops what
the fuzzers do not use).
