# Klinx example workspace

`pipelines/` is a ready-to-open sample [Clinker](https://github.com/rustpunk/clinker)
workspace for klinx. It is a real on-disk workspace tree — a `kiln.toml`
manifest, top-level pipeline `*.yaml` files, reusable `compositions/`,
per-tenant `channels/` overlays, a `retract-demo/` showing relaxed-CK
retraction, and CSV inputs under `data/` — so you can exercise klinx's
`Open Workspace` → `Open File` flow against genuine pipeline documents rather
than the in-app template gallery alone.

## Open it

```bash
dx serve --package klinx
```

Then in klinx:

1. **Open Workspace** → select `examples/pipelines/`.
2. **Open File** → e.g. `customer_etl.yaml` (a small CSV ETL pipeline), or
   `order_fulfillment.yaml`, `audit_join.yaml`, `multi_source_session.yaml`,
   the windowing demos (`tumbling_clicks.yaml`, `hopping_sliding_5m_1h.yaml`),
   or `retract-demo/pipeline.yaml`.

Every pipeline `*.yaml` here is parse-checked against klinx's pinned engine by
`test_vendored_example_pipelines_parse_against_engine` (in
`crates/klinx/src/template.rs`), so these samples stay openable across engine
`rev` bumps.

## Generated artifacts

Running a pipeline writes outputs (`output/`) and run state
(`.kiln-state.json`). These are git-ignored (see `pipelines/.gitignore`) and are
not part of the vendored sample.
