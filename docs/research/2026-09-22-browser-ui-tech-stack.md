# Browser UI tech stack for Klinx (enterprise, Rust-first)

Date: 2026-09-22. Status: research input. The decision is recorded in [docs/ai/decisions/0001-browser-ui-architecture.md](../ai/decisions/0001-browser-ui-architecture.md), which supersedes the recommendation below after two later constraints: no paid dependencies, and long-term stability weighted over reuse. Its fileshare sections are also superseded: workspaces are git repositories on a git host.

## Recommendation

Build a **Rust (Axum) server that Klinx owns, with a Dioxus 0.7 web (WASM) client that reuses the existing components**. The server holds every privileged operation: workspace file IO, git, search, file watching, and Clinker compile. Dioxus 0.7 server functions are plain Axum handlers ([Dioxus server functions docs](https://dioxuslabs.com/learn/0.7/essentials/fullstack/server_functions/)), so use them only as a typed client for an HTTP/JSON API with explicit, stable routes. Keep auth, RBAC and audit as Axum/tower layers that don't depend on Dioxus. The desktop build stays: both builds render the same components behind a `WorkspaceBackend` trait, with a local implementation for desktop and a remote one for the browser. This keeps about 23k lines of UI code and the Rust-first requirement. The stable, framework-neutral API is also a hedge against the biggest risk. Dioxus is pre-1.0 and its team [joined Cognition on 2026-09-10](https://cognition.com/blog/welcoming-dioxus). If Dioxus stalls, only the client layer needs replacing, and the React comparator becomes the fallback. Leptos is not recommended: in May 2026 its author announced it [will be "lightly maintained going forward"](https://github.com/leptos-rs/leptos/issues/4707), and adopting it would mean a full UI rewrite.

## What the current code implies (measured 2026-09-22)

| Surface | Size | Web impact |
|---|---|---|
| `crates/klinx/src` total | 48,941 Rust lines | — |
| `components/` (UI) | 22,878 lines | Kept by Dioxus web; rewritten by Leptos/React/egui |
| `assets/klinx.css` | 9,385 lines | Kept by any DOM-based option |
| `pipeline_view.rs` + `pipeline_view/` (view-model) | ~15.8k lines | Pure Rust; kept by all Rust options; React would need it ported to TS or shipped as WASM |
| Non-test fs/process/notify/rfd call sites | ~43 sites in 9 files: `workspace.rs` (14), `file_ops.rs`, `file_explorer/model.rs`, `search.rs`, `template.rs`, `fs_watcher.rs`, and three UI files that call `git` through `Command::new` directly (`status_bar.rs`, `version_mode/stash_tab.rs`, `version_mode/conflicts_tab.rs`), bypassing `GitOps` | Move to the server |
| `dioxus::desktop` window APIs | 4 files (`main.rs`, `app.rs`, `keyboard.rs`, `title_bar.rs`) | Gate behind the `desktop` feature |
| `document::eval` | 1 call (`yaml_sidebar/panel.rs`) | Replace with `web-sys` for strict CSP |
| `crates/klinx-git` | 1,108 lines, git CLI via `tokio::process` | Server-side only |
| Compile hook (`hooks/compiled_plan.rs`) | `spawn_blocking(compile_active(&config, &root, pipeline_dir))` reads workspace files | Server-side (or a virtual FS in WASM) |

`sync.rs` and `state.rs` are mostly pure (their IO is in tests), so the IO surface is small and already clustered in a few modules.

**Engine crates on `wasm32-unknown-unknown`** (`cargo check` of the local `../clinker` checkout, 2026-09-22): `clinker-core-types`, `clinker-plan` and `clinker-lineage` compile as-is. `cxl` and `clinker-record` fail only because `getrandom` needs the `wasm_js` cfg (via `ahash`), a small fix. `clinker-channel` and `clinker-exec` fail on `uuid` randomness and `errno`/`fs4`, and `exec` also depends on `rayon`, `tempfile` and `libc`, so they belong on the server. **Hypothesis:** "compiles" is not "works". `clinker-plan` uses `walkdir`/`std::fs` to resolve workspace files, so running it client-side needs an in-memory workspace snapshot.

## Comparison

| | **Dioxus 0.7 web + Axum** (recommended) | **Leptos + Axum** | **Axum + TS/React** | egui/eframe web | Dioxus LiveView |
|---|---|---|---|---|---|
| Latest stable / cadence | 0.7.10 (2026-07-30); 0.7.x patches about monthly; 0.8.0-alpha.1 (2026-07-31) | 0.8.19 (2026-06); 0.9 beta | React: very mature | active | 0.7.9 crate |
| 1.0 / LTS | No / No | No / No | Yes (React) | No | No |
| Funding / bus factor | Team inside Cognition since 2026-09-10; no SLA | Solo author; "lightly maintained" | Meta + huge community; React Flow sells paid Pro support | Rerun-backed (not assessed in depth) | Deprioritized |
| Commercial support | None offered | None | Third-party vendors plus library subscriptions | — | — |
| Reuses existing UI code | ~23k component lines + 9.4k CSS | CSS only | CSS only | None | Most (components run server-side) |
| Desktop build remains | Yes, same components | Separate app | Separate app | Yes | Yes |
| Accessibility (WCAG) | DOM-based; proven AA in the field (below) | DOM-based | Best tooling (React Aria and similar) | **Fails on web**: eframe drops the AccessKit tree | DOM |
| Strict CSP | `'wasm-unsafe-eval'` needed; eval/inline removed upstream (below) | Nonce support | Straightforward | wasm | websocket |
| Code editor | CodeMirror 6 via `dioxus_codemirror` (young) or the current textarea | JS interop | Monaco/CodeMirror native | Hard to embed | eval-based |
| Hiring | Rust + niche framework | Rust + niche framework | Largest pool | Niche | Niche |
| Verdict | **Recommend** | Reject | Viable fallback; breaks Rust-first UI | Drop | Drop |

## Per-option findings

### 1. Dioxus 0.7 web client with an Axum server (recommended)
- **Maturity.** 0.7.0 shipped 2025-10-31. Patches followed through 0.7.9 (2026-05-08) ([crates.io history](https://crates.io/crates/dioxus)) and 0.7.10 (2026-07-30). 0.8.0-alpha.0 (2026-05-19) warns of "a number of breaking changes to internal APIs", including `#[non_exhaustive]` props ([releases](https://github.com/DioxusLabs/dioxus/releases)), so plan a 0.7 to 0.8 migration. Klinx pins `=0.7.4`.
- **Ownership.** The team joined Cognition on 2026-09-10. Cognition says it will "continue support for Dioxus, Blitz, Taffy, and Subsecond" and invest more in Dioxus-Native ([Cognition, 2026-09-10](https://cognition.com/blog/welcoming-dioxus)). The announcement gives no term, SLA or governance change, and Nico Burns is named as working on Dioxus full time ([Unite.AI, 2026-09-10](https://www.unite.ai/cognition-adds-dioxus-team-to-advance-devin-coding-agent/); [analysis, 2026-09-14](https://www.beri.net/article/cognition-dioxus-acquihire-blitz-taffy-crate-owners-kitesurf-dependency)). Before this the company had raised about $3.5M (YC, Khosla). This is better funded than Leptos, but priorities are now set by a vendor whose product is Devin.
- **Fullstack.** 0.7 rebuilt server functions on Axum 0.8: any Axum handler can be a server function, with websockets, SSE, streaming and multipart support, plus custom routers and layers ([0.7 release](https://dioxuslabs.com/blog/release-070/); [Axum router docs](https://dioxuslabs.com/learn/0.7/essentials/fullstack/axum/)). Routes are explicit (`#[get("/api/...")]`), so non-Dioxus clients can call the same API.
- **Enterprise evidence.** A public-sector ADR reports a Dioxus 0.7 fullstack portal meeting WCAG 2.1 AA, with 279 E2E tests and zero axe-core violations. It runs a strict CSP where `'wasm-unsafe-eval'` is the only unsafe directive ([Canopy ADR-008](https://canopy-c1fab5.gitlab.io/canopy/adrs/adr-008-applicant-portal-architecture.html)). That is one team's account (**Hypothesis** for Klinx until audited). CSP work landed upstream: inline scripts and the Function constructor were removed ([PR #4310, merged 2025-06-26](https://github.com/DioxusLabs/dioxus/pull/4310)), and so was `eval` on web ([PR #5313, merged 2026-02-16](https://github.com/DioxusLabs/dioxus/pull/5313)). **Open question:** is #5313 in the 0.7.x line Klinx uses, or only in 0.8? Klinx's own `document::eval` call should become `web-sys` either way.
- **IDE needs.** The canvas is SVG/DOM (`components/canvas/*`), so it ports as-is. For large graphs, viewport culling is needed on any DOM stack; React Flow has the same issue ([xyflow #5442](https://github.com/xyflow/xyflow/issues/5442)). On the editor: `dioxus_codemirror` 0.3 wraps CodeMirror 6, vendors its assets (no CDN, which suits air-gapped installs) and bridges LSP, but it is young and drives the editor through `document::eval` ([crate](https://crates.io/crates/dioxus_codemirror)). The current custom textarea editor (1.5k lines) can ship first. CodeMirror 6 takes a CSP nonce ([analysis](https://github.com/cloudfoundry/stratos/issues/5705)); Monaco has open CSP problems for enterprises ([monaco-editor #4927](https://github.com/microsoft/monaco-editor/issues/4927)). **Bundle size:** the Dioxus docs show a TodoMVC build going from 2.36 MB to 234 KB with size optimizations ([Optimizing](https://dioxuslabs.com/learn/0.7/guides/tips/optimizing/)). Klinx will be much larger (**Hypothesis:** several MB with engine crates linked in), so keep the engine out of the first client and use 0.7's WASM-split for lazy loading.
- **Toolchain.** wasm-bindgen moved to its own org with new maintainers, including people from Cloudflare, after the rustwasm org was archived ([Inside Rust, 2025-07-21](https://blog.rust-lang.org/inside-rust/2025/07/21/sunsetting-the-rustwasm-github-org/)).

### 2. Leptos + Axum
The technology is solid: Axum 0.8 integration, websocket server functions ([v0.8.0](https://github.com/leptos-rs/leptos/releases/tag/v0.8.0)), recent security hardening in the 0.8.19/0.9-beta line, and `Nonce` support for CSP ([releases](https://github.com/leptos-rs/leptos/releases)). But in May 2026 the author wrote that Leptos "will be lightly maintained going forward" and invited other maintainers to step in ([#4707](https://github.com/leptos-rs/leptos/issues/4707)). Open Collective income is about $157 in total ([Open Collective](https://opencollective.com/leptos)). Klinx would pay a full rewrite of 23k UI lines to move onto a lower-support framework. **Reject.**

### 3. Axum + TypeScript/React (comparator)
This has the deepest ecosystem for enterprise UI: accessible component libraries, Monaco and CodeMirror as first-class options, and React Flow, which offers paid Pro support with an enterprise procurement tier ([React Flow Pro](https://reactflow.dev/pro)). It also has the largest hiring pool; React remains the most-used web framework in the [2025 Stack Overflow survey](https://survey.stackoverflow.co/2025/technology). The costs: all 23k component lines are rewritten, and the ~15.8k-line `pipeline_view` view-model has to be either ported to TS (logic would drift from the engine) or compiled to WASM and called from TS. Klinx would also maintain two languages and two build chains, and lose the shared desktop build. The Axum backend is the same as in option 1, which is why it's the natural fallback.

### 4. Other options (dropped)
- **egui/eframe on the web.** It renders to a canvas, and eframe discards the AccessKit tree on the web (`accesskit_update: _, // not currently implemented`), so screen readers get nothing without a third-party DOM-mirror plugin ([egui-reactor-app a11y docs](https://docs.rs/egui-reactor-app/latest/egui_reactor_app/a11y/index.html)). Web text input and IME are still being fixed ([egui #8068](https://github.com/emilk/egui/pull/8068)). It fails WCAG. **Drop.**
- **Dioxus LiveView (thin client, all Rust on the server).** Maintainers are "deprioritizing liveview as a platform and may drop support" ([discussion #3378](https://github.com/DioxusLabs/dioxus/discussions/3378)). **Drop.**
- Tauri and Dioxus Native don't target browsers. Yew and Sycamore were not assessed.

## Recommended architecture

```
Browser (WASM, Dioxus web)            klinx-server (Axum, one container)                Storage
  components/*  (shared)  ── HTTPS ─▶  tower layers: OIDC session → RBAC → audit log   ┌─ Model A: SMB/NFS share mounted
  RemoteBackend (typed API)  JSON      /api/workspace/*  fs service (cap-std confined)─┤   in the pod
  SSE/websocket ◀── file events ────   /api/git/*        GitOps (git CLI, sandboxed)  └─ per-user scratch clones (PVC)
  optional: plan compile in WASM       /api/compile      clinker-plan/lineage/exec
                                       /api/search, /events (notify → SSE)
Desktop (wry, same components) ── LocalBackend (in-process; today's code path)
```

- **Crate split.** A pure `klinx-core` holds `pipeline_view`, `sync`, the state models and the YAML patching. `klinx-api` holds the DTOs and the `WorkspaceBackend` trait. `klinx` becomes the UI with `desktop`/`web` features, `klinx-server` is the Axum binary, and `klinx-git` stays as is. The three direct `git` call sites move behind `GitOps` first.
- **Auth.** Use OIDC authorization-code + PKCE in Axum (`openidconnect` / [`axum-oidc`](https://docs.rs/axum-oidc)) with server-side sessions. Get SAML by federating through the customer IdP or a broker such as Keycloak rather than in-process. [`samael`](https://docs.rs/crate/samael/latest) is 0.0.x, described as "a work in progress", and needs xmlsec C libraries. A pure-Rust `saml` crate exists but has no track record (**Hypothesis**). For RBAC, candidates are Cedar or Casbin; neither was evaluated.
- **Fileshare boundary.** Only the server touches the share. Every path is resolved under the workspace root with a capability handle, which rejects `..` and symlink escapes. Klinx does not rely on string canonicalization alone.
- **Git boundary.** The git CLI runs as a child process with a scrubbed environment (`GIT_CONFIG_NOSYSTEM`, `GIT_TERMINAL_PROMPT=0`, hooks disabled, explicit `safe.directory`), timeouts and output caps. Commit author comes from the OIDC identity. Pushing uses a per-user token or a service credential (**Open question**). Stay on the CLI: gitoxide still lacks push, checkout, merge and stash ([crate-status](https://github.com/GitoxideLabs/gitoxide/blob/main/crate-status.md)).
- **Concurrency.** YAML text stays authoritative. Saves are conditional on the file's blake3 hash (ETag/If-Match), with a conflict UI when the hash doesn't match. Per-file soft locks are optional. Real-time co-editing (CRDT) is out of scope for v1.
- **Operations.** One static binary plus WASM assets, no CDN, packaged as a container and Helm chart. An offline bundle covers air-gapped installs. Observability is `tracing` exported over OpenTelemetry. SBOMs come from `cargo deny` plus CycloneDX. The web target also makes Playwright + axe testing possible, which the desktop-only app never allowed.

### How each fileshare model is handled

| | Model A: share mounted on the server | Model B: files on each user's machine |
|---|---|---|
| Dioxus + Axum (rec.) | Works as sketched. Identity: either (A1) the service account mounts the share and Klinx RBAC maps groups to workspaces (simple, audited in-app, but share ACLs are bypassed), or (A2) per-user access through SMB `multiuser` Kerberos mounts or NFSv4 krb5 with constrained delegation (enforces share ACLs, heavy AD work; **Hypothesis** on feasibility in k8s) | A browser can't open local folders portably: `showDirectoryPicker` is not supported in Firefox or Safari ([caniuse](https://caniuse.com/mdn-api_window_showdirectorypicker)), and git can't run in the browser. Serve these users with the **desktop build** (same components, `LocalBackend`), or ship `klinx-server` as a localhost agent the browser connects to |
| Leptos | Same server design | Separate desktop app, or a localhost agent |
| React | Same server design | Localhost agent, or keep the Dioxus desktop app (two UIs) |

Git on SMB/NFS working trees has lock-file and performance risks. The safer pattern keeps a bare or remote repo as the source of truth and gives each user a clone on server-local storage (**Hypothesis**; validate against the target share).

## Risks
1. **Framework stewardship.** Dioxus is pre-1.0, 0.8 brings breaking changes, and the team's primary job is now Devin. There is no commercial support contract. Mitigation: pin 0.7.x, keep the API framework-neutral, keep UI logic in `klinx-core`, budget time for the 0.8 migration, and treat React as a costed fallback.
2. **Fileshare identity and git on shares.** Enforcing per-user share ACLs from a server, and running git safely against network filesystems, is the hardest part of the project and can't be settled until the fileshare model is chosen.
3. **Enterprise compliance gaps.** Rust SAML is immature, so federate. The custom textarea editor and the SVG canvas have never been accessibility-audited, and WCAG remediation is real work in any stack. Strict CSP depends on removing `eval` (upstream status for 0.7.x unverified).
4. Minor: latency of server-side compile on each debounced edit (**Hypothesis**: acceptable over websocket; measure), and WASM bundle size if engine crates move client-side.

## Open questions (user decisions)
1. **Fileshare model**: server-mounted share (A) or files on each user's machine (B)? If A, which option: service account with app RBAC (A1), or per-user Kerberos pass-through (A2)? Is the share the git working tree, or is there a central git remote?
2. Identity provider and protocol (OIDC available, or SAML-only?), and which groups map to which roles.
3. Deployment target: customer-hosted Kubernetes, VMs, or SaaS? Are air-gapped installs required?
4. Concurrency expectation: last-writer-with-conflict-detection, file locks, or live co-editing?
5. Must the desktop app stay a supported product, or only a fallback for Model B?
6. Is a vendor support contract for the UI framework a hard requirement? If yes, React wins on that criterion.
7. Target browsers and WCAG level (2.1 AA or 2.2 AA).
8. Whose credentials push to git remotes: per-user tokens or a service identity?
9. Do run/preview features (`clinker-exec`) execute on the server, and under what resource limits?

## Phased migration
0. **Decide and spike (about 2 weeks).** Answer open questions 1–3. Build the current UI with the `web` feature and a stub backend. Measure WASM size, CSP and first paint.
1. **Seam extraction (no behavior change).** Split out `klinx-core` and `klinx-api` and add `WorkspaceBackend` with a `LocalBackend`. Route the direct `git` calls through `GitOps`. Gate `dioxus::desktop`, `rfd` and `notify` behind the `desktop` feature. Desktop keeps shipping.
2. **Server.** Build `klinx-server` on Axum: confined fs, git, search, compile, events over SSE/websocket, OIDC sessions, audit log.
3. **Web client.** Add a `RemoteBackend`, an in-app workspace picker in place of native dialogs, `web-sys` in place of `eval`, and a strict CSP. Add Playwright + axe tests to CI.
4. **Enterprise hardening.** RBAC, conditional saves and conflict UI, OpenTelemetry, Helm chart and offline bundle, SBOM, WCAG audit and fixes.
5. **Optional.** Client-side `clinker-plan`/`lineage` compile in WASM for latency, a CodeMirror 6 editor, live co-editing, and the Dioxus 0.8 upgrade.
