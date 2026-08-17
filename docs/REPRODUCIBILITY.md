# Build reproducibility

The PRD asks for reproducible builds *or* an honest explanation of why not. This is the
explanation, plus what is actually guaranteed today.

## What is guaranteed

**Build provenance.** Every release artifact carries a signed
[SLSA provenance attestation](https://slsa.dev/) produced by
`actions/attest-build-provenance` in `.github/workflows/release.yml`. That is a
cryptographic statement that *this specific binary was built by that workflow, from this
repository, at that commit*.

```bash
gh attestation verify neobrowser-aarch64-apple-darwin.tar.gz --repo pitiflautico/neobrowser
```

This is a stronger practical guarantee than a checksum. A checksum proves the file
matches a hash published next to it — which an attacker who replaced the file would also
control. Provenance proves the build's origin.

**An SBOM per release** (`neobrowser-sbom.cdx.json`, CycloneDX), so "does this ship the
crate that just got a CVE" is answerable without rebuilding.

**A locked dependency graph.** `Cargo.lock` is committed, and CI runs `cargo audit` and
`cargo deny` on every push. That is what caught a real advisory in `rustls-webpki` on the
first run of those checks.

## What is not guaranteed, and why

A byte-identical rebuild is **not** currently guaranteed. The specific obstacles, so this
can be revisited rather than remaining a vague disclaimer:

1. **Absolute paths in debug info.** `rustc` embeds the build directory. The release
   profile sets `strip = true`, which removes most of it, but `panic!` messages still
   carry `file!()` paths relative to the build root. Fixable with
   `--remap-path-prefix`; not yet done because it needs verifying that panic messages
   stay useful afterwards.
2. **`rusqlite` with the `bundled` feature compiles SQLite from C.** The output depends
   on the C compiler version on the runner, which GitHub upgrades without notice. Pinning
   that would mean pinning a container image for the build.
3. **Toolchain drift.** The workflow uses `dtolnay/rust-toolchain@stable`, so a release
   built today and rebuilt in three months uses a different compiler. A
   `rust-toolchain.toml` pin would fix it at the cost of not getting compiler
   improvements or fixes automatically.

None of these is hard to solve; they are unsolved because provenance already covers the
threat that reproducibility is usually reached for (*is this binary really from this
source?*), and the remaining benefit is defence against a compromised builder.

## If you want to verify by rebuilding

You will get a functionally identical binary, not a bit-identical one:

```bash
git clone https://github.com/pitiflautico/neobrowser && cd neobrowser/rust
git checkout v0.1.7
cargo build --release --locked      # --locked: fail rather than silently updating deps
./target/release/neobrowser --version
./target/release/neobrowser doctor --json
```

Comparing behaviour rather than bytes: `cargo test` runs the full suite, including the
live-Chrome integration tests and the property/fuzz suite. `cargo test --test conformance`
additionally checks the scenarios in [VERIFIED-ACTIONS.md](VERIFIED-ACTIONS.md), which is the
behavioural claim most worth reproducing independently — it needs a real Chrome and self-skips
without one, and a skip is not a pass.
