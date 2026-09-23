# 0001: Browser UI architecture — Rust core and server, React UI, Tauri desktop

- **Status:** Accepted (direction). The UI framework choice is confirmed by the phase 2 gate below before any UI rewrite starts.
- **Date:** 2026-09-22
- **Evidence:** [docs/research/2026-09-22-browser-ui-tech-stack.md](../../research/2026-09-22-browser-ui-tech-stack.md)

## Context

Klinx is a Dioxus 0.7 desktop app (wry/WebKitGTK). The goal is a browser UI with workspace access over a fileshare, suitable for enterprise deployment, with Rust as the primary language.

Constraints:

- **Open source with no paid dependencies.** Paid libraries, paid tiers, and support contracts are excluded. "Enterprise ready" means a free license, a stable release history, healthy stewardship, and a large community.
- **Contributor reliability.** Prefer stacks with stable APIs, deep documentation, and a large ecosystem, where contributors and tooling produce correct code with fewer review rounds.
- **Long-term stability and feature support outweigh reuse** of the current UI code.

Relevant findings, from the research report unless noted:

- No Rust web UI framework is 1.0 or has stable stewardship. Dioxus is pre-1.0, its 0.8 line has breaking changes, and its team joined Cognition on 2026-09-10 with no public support commitment. The Leptos author announced in May 2026 that it will be "lightly maintained". egui fails WCAG on the web. Dioxus LiveView is being deprioritized.
- React/TypeScript has the deepest free ecosystem for this app: React Flow (MIT) for the canvas, CodeMirror 6 (MIT) for the editor, React Aria (Apache-2.0) for accessible components, and Playwright + axe-core for testing. **Strong inference:** contributors and tooling produce correct React/TypeScript more reliably than Dioxus 0.7, given documentation depth and API stability; Dioxus 0.7 contributions already need project-specific pattern and antipattern guidance.
- Tauri 2 (MIT/Apache-2.0) is a stable Rust desktop shell. Its steward announced in June 2026 that it is moving Tauri to a foundation.
- The desktop-only surface is small and clustered: about 43 non-test fs/process/dialog call sites in 9 files, 4 files using `dioxus::desktop` APIs, and `klinx-git`. Three UI files shell out to `git` directly instead of going through `GitOps`.
- `clinker-core-types`, `clinker-plan`, and `clinker-lineage` compile for `wasm32-unknown-unknown`. `cxl` and `clinker-record` need a `getrandom` configuration fix. `clinker-channel` and `clinker-exec` are server-side only.

## Decision

1. **Rust owns all domain logic.**
   - `klinx-core` (pure Rust): the `pipeline_view` view model (canvas derivation, field lineage, layout), YAML patching, and sync. It compiles natively for the server and desktop, and to WASM for the browser, so lineage and layout logic are never re-implemented in TypeScript.
   - `klinx-api`: request/response types and a `WorkspaceBackend` interface covering workspace files, git, search, compile, and file events.
2. **`klinx-server` (Axum)** holds every privileged operation. Workspace file IO is confined to the workspace root with capability handles. Git runs via the CLI behind `GitOps` in a sandboxed child process. The server also handles search, Clinker compile, file events over SSE or websocket, OIDC sessions (SAML only through federation, e.g. Keycloak), role-based access, and an audit log. Only the server touches the fileshare.
3. **UI: React + TypeScript**, rendering view models from `klinx-core` (WASM) and calling `klinx-api`. TypeScript is limited to presentation.
4. **Desktop: Tauri 2** runs the same React UI through a local, in-process `WorkspaceBackend`. It is also the supported path for users whose files are on their own machine, since browsers cannot open local folders portably.
5. **YAML text stays authoritative.** Saves are conditional on the file's content hash, with a conflict UI. Live co-editing is out of scope.

### Phase 2 gate (confirms step 3)

At the end of phase 2, build a spike: the canvas and inspector in React + `klinx-core` WASM against `klinx-server`, compared with the Dioxus web build of the same screens. Compare:

- WASM/JS bundle size and first paint;
- canvas interaction performance on the large-pipeline fixture;
- axe-core accessibility results;
- implementation effort and review rounds.

React proceeds unless the spike shows a blocking regression. The result is recorded as an amendment to this decision.

## Consequences

- About 23k lines of Dioxus components are rewritten. Most of the CSS carries over, and the current app serves as a behavioral and visual reference.
- Two languages and build chains (Cargo plus a JS toolchain). CI gains Playwright + axe tests, which the desktop-only app never supported.
- The Dioxus version pin and the Dioxus-specific contributor guidance stop mattering once the rewrite completes. Until then the desktop app keeps shipping unchanged.
- Engine integration work (for example the Clinker pin bump and OpenLineage work) stays in Rust and is unaffected.
- Phases 1 and 2 are framework-neutral, so they carry no regret if the gate reverses step 3.

## Phases

0. **Decide** the open questions below.
1. **Seam extraction** (#212) (no behavior change): `klinx-core`, `klinx-api`, a `WorkspaceBackend` with a local implementation, direct `git` calls routed through `GitOps`, and desktop-only APIs behind a `desktop` feature.
2. **`klinx-server`** (#213): confined file access, git, search, compile, events, OIDC, roles, audit. Ends with the gate spike.
3. **React UI** against `klinx-core` WASM and `klinx-api`, with Playwright + axe in CI.
4. **Tauri desktop** on the React UI; retire the Dioxus app.
5. **Hardening**: conflict UI, OpenTelemetry, container/Helm packaging and an offline bundle, SBOM, WCAG audit.

## Open questions

Tracked in [80_OPEN_QUESTIONS.md](../80_OPEN_QUESTIONS.md):

- Fileshare model — **answered 2026-09-22** (see Amendments): a server-mounted share, and workspaces are git working copies on server-local disk whose remote may be a git server, a bare repo on the share, or S3. Share access identity — **answered 2026-09-22** (see Amendments): service account first, with per-user identity phased in.
- Identity provider and protocol (OIDC available, or SAML-only?).
- Deployment target, and whether air-gapped installs are required.
- Whose credentials push to git remotes: per-user tokens or a service identity?

## Amendments

### 2026-09-22: Storage model

- Workspaces live on a fileshare **mounted on the server** (Model A). The browser never accesses storage directly, so the Tauri desktop app is no longer needed as the path for local-file users. It stays in scope as a desktop build of the same UI.
- **Workspaces are git working copies on the server's local disk.** Their remote is any git URL: a git server (GitHub, GitLab, Forgejo), a bare repository on the mounted share, or an S3 bucket through [git-remote-s3](https://github.com/awslabs/git-remote-s3) (Apache-2.0). That helper takes a per-branch lock with S3 conditional writes and rejects a stale push with "fetch and retry", matching normal git push semantics. Nobody works directly in the share or the bucket: a git working tree on an S3 filesystem mount is not viable, because git needs atomic renames and lock files (**Strong inference**). Klinx therefore talks only git for workspace storage and needs no object-store layer of its own. Caveats of S3 remotes: a Python helper in the server image, conditional-write support in S3-compatible stores, and no pull-request flow.

### 2026-09-22: Share access identity

The server accesses storage as a **service account**, with authorization enforced in klinx and an append-only audit log. Storage and remote credentials sit behind an interface keyed by (user, workspace), so stronger identity models slot in later without redesign:

1. Service account; OIDC group claims map to workspace roles; audit log of every mutating operation.
2. Narrower blast radius: per-workspace mounts or service accounts selected from IdP groups.
3. Per-user credentials where the remote supports them: per-user tokens for git servers, short-lived per-user credentials for S3 via OIDC federation.
4. Optional per-user Kerberos delegation backend for VM or single-host installs, built only on demand.

Why not per-user Kerberos first:

- OIDC login yields no Kerberos ticket, so it needs protocol transition (S4U2Self/S4U2Proxy), which AD security tooling flags as unsecure.
- Comparable products (Nextcloud, Posit Workbench) document that their per-user share modes do not work with SAML/OIDC. Git hosts (Gitea, Forgejo, GitLab) use one service identity plus app authorization.
- The delegating key can impersonate every delegation-enabled user, so a server compromise exposes about as much as a service account does.
- Kernel CIFS mounts need elevated container privileges and per-user uids and ticket caches, and tickets expire mid-operation.

Per-user delegation becomes worth building if a deployment requires the file server's own audit log to name users, if users also reach the share directly (share ACLs must be the only authority), or for single-host installs where AD admins approve delegation.
