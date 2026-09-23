# 0001: Browser UI architecture — Rust core and server, React UI

- **Status:** Accepted (direction). The UI framework choice is confirmed by the phase 2 gate below before any UI rewrite starts.
- **Date:** 2026-09-22
- **Evidence:** [docs/research/2026-09-22-browser-ui-tech-stack.md](../../research/2026-09-22-browser-ui-tech-stack.md)

## Context

Klinx is a Dioxus 0.7 desktop app (wry/WebKitGTK). The goal is a browser UI whose workspaces are git repositories on a git host, suitable for enterprise deployment, with Rust as the primary language.

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
2. **`klinx-server` (Axum)** holds every privileged operation: per-user git working copies on server-local disk (confined to the workspace root with capability handles), git through the CLI behind `GitOps` in a sandboxed child process, search, Clinker compile, file events over SSE or websocket, OIDC sessions (SAML only through federation, e.g. Keycloak), role-based access, and an audit log. The browser never touches storage or git directly.
3. **UI: React + TypeScript**, rendering view models from `klinx-core` (WASM) and calling `klinx-api`. TypeScript is limited to presentation.
4. **Desktop: Tauri 2** runs the same React UI through a local, in-process `WorkspaceBackend`. *Superseded by the 2026-09-22 amendment "No desktop app".*
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
4. **Retire the Dioxus desktop app** once the web UI reaches parity (no Tauri port; see Amendments).
5. **Hardening**: conflict UI, OpenTelemetry, container/Helm packaging and an offline bundle, SBOM, WCAG audit.

## Open questions

Tracked in [80_OPEN_QUESTIONS.md](../80_OPEN_QUESTIONS.md):

- Identity provider and protocol for signing in to klinx (OIDC available, or SAML-only?), and whether the git host may serve as the sign-in provider.
- Deployment target (Kubernetes, VMs), and whether air-gapped installs are required.
- Whether installs without a git host need S3 as a git remote (see Amendments).
- Name and location of the per-repository promotion configuration.
- Whether background jobs run with no user present, and under which identity.

## Amendments

### 2026-09-22: Workspaces are git repositories on a git host

- **A workspace is a git repository hosted on a git host** (GitHub, Bitbucket, GitLab, Forgejo, or similar). Fileshares are not a workspace storage model. Version control stops being optional, unlike today's desktop app, which disables its git features for non-git folders. A workspace may be a subdirectory of a larger repository (as `examples/pipelines` is). **Open question:** whether the hosted model requires a repository root.
- **The server keeps per-user working copies** on its local disk: clone on first open, then pull, commit, and push to the host. Saves write the working copy; commits and pushes are explicit user actions. Nobody edits the host's repository directly.
- **Access uses each user's own git host credentials.** A user links their git host account (OAuth on GitHub, Bitbucket, and GitLab), and klinx clones, commits, and pushes as that user, so the host's repository permissions, branch protection, review, and audit apply to each user without klinx duplicating them. Tokens are stored encrypted server-side, scoped to repository access, and refreshed or revoked with the host. Credentials sit behind an interface keyed by (user, workspace), so a service identity can serve read-only or background work where a deployment needs it. Signing in to klinx itself stays OIDC; the git host may double as the sign-in provider where it offers OIDC.
- **Pull requests** go through the host. Klinx detects GitHub, GitLab, and Bitbucket remotes today but only creates pull requests on GitHub (via the `gh` CLI), so Bitbucket and GitLab pull-request support is follow-up work.
- **S3 as a git remote** via [git-remote-s3](https://github.com/awslabs/git-remote-s3) (Apache-2.0) remains possible for installs without a git host: it locks per branch with S3 conditional writes and rejects stale pushes like a normal git server. It has no pull-request flow and puts a Python helper in the server image. **Open question:** whether any deployment needs it.

### 2026-09-23: Team workflow, promotion, and deployment

- **A team owns its workspace repository**: pipelines, channels, compositions, and schemas. Ownership, required reviewers (e.g. `CODEOWNERS`), and branch protection are configured on the git host, which stays the authority; klinx surfaces them and does not re-implement them.
- **Individuals change files on their own branch.** Klinx creates a branch from the target branch, keeps the user's working copy, and lets them edit only the files they need, test against data locations (below), commit, push, and open a pull request. Several people work in the same repository at once without sharing a working copy.
- **Pull requests run the host's CI.** Klinx shows each pull request's review state and check results (e.g. Clinker validation of changed pipelines in the team's CI) and links to the host; it does not run CI itself.
- **Deploy means merging into a configured branch.** External CI/CD tools deploy from those branches; klinx has no deployment step of its own.
- **Promotion across branches is configurable and optionally required.** A team declares its branch chain per repository (for example `dev` → `staging` → `prod`, or just `main`) in a committed configuration file, so the policy is itself reviewed and versioned. Each promotion is a pull request from one branch to the next. A team can require promotion in order (no change reaches `prod` except from `staging`) or allow direct targeting. Klinx guides and checks promotions in its UI; the host's branch protection is what enforces them. **Open question:** the configuration file's name and location (a klinx-specific file, or a section in `clinker.toml`).

### 2026-09-23: Data locations

- **Pipeline data is separate from the workspace.** Test data and the files Clinker sources read and sinks write may live on a fileshare mounted on the server, in S3, or in a hosted repository. The workspace itself is always a git repository.
- **The server reaches data locations with a service account**, with authorization enforced in klinx and an append-only audit log, behind a credential interface keyed by (user, data location). Later phases can narrow blast radius (per-location mounts or accounts chosen from IdP groups), use per-user short-lived S3 credentials via OIDC federation, and add optional per-user Kerberos delegation for single-host installs. Per-user Kerberos first was rejected: OIDC sign-in yields no Kerberos ticket, so it needs protocol transition that AD security tooling flags as unsecure; comparable products document that their per-user share modes do not work with SAML/OIDC; the delegating key can impersonate every delegation-enabled user; and kernel CIFS mounts need elevated container privileges while tickets expire mid-operation. Per-user delegation becomes worth building if a deployment requires the file server's own audit log to name users, or if share ACLs must remain the only authority.
- Clinker's own S3 source/sink support is tracked in the Clinker repository.

### 2026-09-22: No desktop app

- **The desktop app is out of scope.** Workspaces live on git hosts reached through the server, so a native app has no storage role. Single-user or offline use runs `klinx-server` on localhost and opens it in a browser (the same code as a hosted install), optionally installed as a PWA for an app-like window. Dropping it avoids per-OS builds, signing, and updates, and a second in-process backend. Tauri can still wrap the same React UI later if a concrete native need appears. The Dioxus desktop app keeps shipping until the web UI reaches parity.

### 2026-09-23: The web version starts as a new project

- **The browser version is built in a new repository, not extracted from the desktop app.** The seam-extraction phase (#212) and the remaining `AppShell` hook work (#14) existed mainly to keep the Dioxus desktop app shipping, unchanged, through the migration. With the desktop app frozen (below), that scaffolding (a local `WorkspaceBackend` implementation, a `desktop` feature gate, screenshot parity) has no remaining purpose.
- **Proven domain logic is imported, not rewritten.** `pipeline_view` (including field lineage and layout), `yaml_patch`, and `sync` (about 17k lines with 162 tests) move into the new project's core crate together with their tests, ideally with history. They carry the product's correctness guarantees (canvas and lineage must not claim anything untrue, and the user's YAML text stays authoritative), and none of them depend on Dioxus. Everything else — server, API types, React UI, repository layout, CI — is designed fresh for the server + WASM + React architecture in this decision.
- **Repositories:** this repository is renamed `klinx-desktop`; the new project takes the `klinx` name. Open issues that concern the web product move to the new repository.
- **The desktop app is frozen** at its current behavior: critical fixes only. Canvas, lineage, and editor backlog work is retargeted to the new project instead of being built twice. This replaces the earlier "keeps shipping until the web UI reaches parity" wording in the Phases section and the 2026-09-22 "No desktop app" amendment.
- **Sequencing:** the in-flight Clinker pin bump (#211) lands in `klinx-desktop` first, so the imported modules start on the current Clinker revision.
- **Unchanged:** decisions 1–3 and 5, the workspace-is-a-git-repository, team-workflow, and data-location amendments, the phase 2 gate spike, and the open questions (identity provider, deployment target, S3 remote, promotion configuration, background jobs).
