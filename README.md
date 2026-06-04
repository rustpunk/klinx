# Klinx

Klinx is the standalone desktop IDE for authoring [Clinker](https://github.com/rustpunk/clinker)
YAML pipeline configurations. It is a [Dioxus](https://dioxuslabs.com) 0.7
application that runs as a native desktop app (`wry` webview) on Linux, macOS,
and Windows.

Klinx provides a visual pipeline canvas, a node inspector, a YAML editor with
CXL syntax support, schema and provenance panels, a git version-control mode,
and a bundled gallery of starter pipeline templates.

## Workspace layout

```
klinx/
  crates/
    klinx/        the IDE binary (Dioxus desktop)
    klinx-git/    git VCS abstraction (CLI-based, future gix upgrade path)
```

## Engine crates come from Clinker via a git pin

Klinx does not vendor the Clinker engine. The five engine crates it consumes —
`cxl`, `clinker-core`, `clinker-record`, `clinker-schema`, and
`clinker-channel` — are declared as git dependencies pinned to a single
Clinker commit:

```toml
cxl = { git = "https://github.com/rustpunk/clinker", rev = "c233a38" }
# ...and the other four, same git + rev
```

The first build fetches that commit from the public Clinker repository, so the
initial `cargo build` needs network access. Subsequent builds use the cached
checkout. Bumping the engine surface means bumping the `rev` in the workspace
`Cargo.toml` `[workspace.dependencies]` block.

## Prerequisites

- Rust 1.91 (pinned in `rust-toolchain.toml`; `clippy` and `rustfmt` components).
- The Dioxus CLI:

  ```bash
  cargo install dioxus-cli
  ```

- For the desktop target on Linux: WebKitGTK and GTK3 development packages
  (`libwebkit2gtk-4.1-dev`, `libgtk-3-dev`, `libxdo-dev` on Debian/Ubuntu).

## Running

```bash
# Desktop target — native wry webview (default platform per Dioxus.toml)
dx serve --package klinx --platform desktop
```

There is no automated UI integration test target: klinx is desktop-only and
the `wry` desktop webview cannot be driven by Playwright or similar headless
browser tools. Correctness validation relies on the cargo test suite and manual
exploratory testing against the desktop build.

## Checks

The CI gauntlet (`.github/workflows/ci.yml`):

```bash
cargo fmt --all --check
cargo clippy --workspace -- -D warnings
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
```

## License

MIT. See [LICENSE](LICENSE).
