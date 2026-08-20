# Plan: Stratagem Hot Update and Admin GUI

**Generated**: 2026-08-20
**Estimated Complexity**: High

## Overview

Add a data-only hot-update channel for the built-in stratagem catalog and deploy a minimal Cloudflare Access-protected administration GUI at `https://update.unsnow.online/admin/`. The administrator edits a stratagem, uploads SVG/PNG/JPEG artwork, previews the client result, and saves once; the backend validates the complete catalog, creates an immutable signed snapshot, advances the public manifest atomically, and records an audit event. Clients remain usable offline, load the last verified catalog immediately, check for a newer catalog after startup, show an update notification, and fall back to the previous or bundled catalog after any failure.

The admin visual direction is restrained Apple-style utility UI: system typography, neutral surfaces, generous whitespace, fine dividers, one blue primary action, no decorative copy, glow, texture, or promotional motion.

## Prerequisites

- Current Rust/Tauri repository based on the latest protected `main`.
- Existing Ubuntu 24.04 server and `update.unsnow.online` Nginx virtual host.
- Cloudflare Zero Trust self-hosted application protecting `/admin/*` with the Cloudflare identity provider.
- Cloudflare Access team domain and application audience tag for origin-side JWT verification.
- A dedicated unprivileged Linux service account and a root-owned deployment directory.
- Ed25519 signing key readable only by the catalog service; public key compiled into the Rust client.

## Sprint 1: Catalog Contract and Safe Publisher

**Goal**: Produce locally runnable, signed, immutable catalog versions without a GUI.

**Demo/Validation**:

- Start the service on loopback with development authentication enabled.
- Read the manifest and current catalog.
- Publish a valid item and confirm the manifest advances only after the immutable snapshot is complete.
- Submit malformed fields and confirm the current manifest does not change.

### Task 1.1: Define the versioned catalog schema

- **Location**: `server/stratagem-admin/lib/catalog.mjs`, `server/stratagem-admin/data/seed-catalog.json`
- **Description**: Define schema version 1, stable IDs, supported groups, bilingual names, aliases, OCR terms, WASD sequence, enabled state, and bundled or normalized icon sources.
- **Dependencies**: None.
- **Acceptance Criteria**:
  - Duplicate IDs, unsupported groups, empty names, invalid directions, unsafe icons, excessive counts, and oversized fields are rejected.
  - Remote IDs cannot use the reserved `custom_` prefix.
  - Disabled items remain resolvable but are hidden from new selections.
- **Validation**: Node unit tests for every validation boundary.

### Task 1.2: Export the bundled client catalog as the server seed

- **Location**: `scripts/export-stratagem-catalog.mjs`
- **Description**: Evaluate the existing static database and emit deterministic JSON referencing bundled icon names.
- **Dependencies**: Task 1.1.
- **Acceptance Criteria**:
  - Exported IDs and item count exactly match the bundled client database.
  - Output is stable across repeated runs.
- **Validation**: Regression assertion comparing the export with `defaultStratagemDB`.

### Task 1.3: Implement immutable publication and audit storage

- **Location**: `server/stratagem-admin/lib/store.mjs`
- **Description**: Serialize writes, assign monotonically increasing versions, sign exact catalog bytes with Ed25519, atomically replace `manifest.json`, keep immutable versions, and append bounded audit records.
- **Dependencies**: Tasks 1.1–1.2.
- **Acceptance Criteria**:
  - Interrupted publication cannot expose a partial snapshot.
  - Stale editor versions receive a conflict response.
  - Rollback publishes a new higher version containing an older snapshot.
- **Validation**: Temporary-directory tests for success, validation failure, stale version, interrupted write, restart recovery, and rollback.

### Task 1.4: Normalize uploaded artwork

- **Location**: `server/stratagem-admin/lib/icons.mjs`
- **Description**: Accept SVG, PNG, and JPEG up to 1 MiB; rasterize with Sharp; fit into the existing 256×256 background and border treatment; emit a static SVG containing only a normalized embedded PNG.
- **Dependencies**: Task 1.1.
- **Acceptance Criteria**:
  - Scripts, external URLs, SVG event handlers, metadata, and malformed images cannot survive normalization.
  - Output has a 256×256 viewBox and bounded encoded size.
- **Validation**: Malicious SVG fixtures, invalid MIME/magic bytes, oversized input, and golden preview tests.

## Sprint 2: Minimal Admin GUI and Cloudflare Access Authorization

**Goal**: Make the complete edit/publish/history workflow usable from a protected browser page.

**Demo/Validation**:

- Open `/admin/` with a valid Cloudflare Access session.
- Add a stratagem, preview the exact normalized icon/card, save, and observe a new public catalog version.
- Disable, restore, and roll back from history.

### Task 2.1: Implement origin-side Cloudflare Access JWT validation

- **Location**: `server/stratagem-admin/lib/access.mjs`
- **Description**: Validate `Cf-Access-Jwt-Assertion` using Cloudflare's remote JWKS, exact issuer, application audience, RS256, expiry, not-before, and optional email allowlist.
- **Dependencies**: None.
- **Acceptance Criteria**:
  - Missing, expired, wrong-audience, wrong-issuer, and invalid-signature tokens return 403.
  - Development bypass is rejected unless the service is bound to loopback and explicitly enabled.
- **Validation**: Generated RSA fixture tokens plus missing-configuration tests.

### Task 2.2: Implement the admin and public HTTP API

- **Location**: `server/stratagem-admin/server.mjs`
- **Description**: Serve the admin UI, session information, current catalog, publish, disable/restore, history, rollback, public manifest, immutable catalog versions, and health endpoint.
- **Dependencies**: Sprint 1 and Task 2.1.
- **Acceptance Criteria**:
  - Only public read endpoints bypass Access.
  - Mutation methods require validated Access identity and same-origin JSON requests.
  - Request bodies, methods, cache headers, and response types are bounded and explicit.
- **Validation**: HTTP integration tests with isolated storage.

### Task 2.3: Build the restrained administration interface

- **Location**: `server/stratagem-admin/public/index.html`, `styles.css`, `app.js`
- **Description**: Build a compact catalog list, editor sheet, exact client card preview, icon picker, publish confirmation, status toast, and version history/rollback view.
- **Dependencies**: Task 2.2.
- **Acceptance Criteria**:
  - System font, neutral palette, fine dividers, visible focus, 44 px actions, one primary action, no ornamental motion or explanatory panels.
  - Desktop and mobile layouts are fully usable with keyboard and screen-reader labels.
  - Save publishes immediately after a concise confirmation and returns the new version.
- **Validation**: Real-browser desktop/mobile screenshots, keyboard path, empty/loading/error/conflict states, reduced-motion check.

## Sprint 3: Rust/Tauri Catalog Consumer

**Goal**: Make the Windows client safely consume the signed catalog without delaying startup.

**Demo/Validation**:

- Launch offline using bundled data.
- Launch with a verified cached catalog and observe it before the network check completes.
- Publish a new catalog, restart the client, accept the notification, and verify the new item, OCR term, shortcut filter, and overlay.
- Serve bad signatures and malformed data and confirm the current catalog remains unchanged.

### Task 3.1: Implement signed manifest/catalog fetching

- **Location**: `src-tauri/src/catalog.rs`, `src-tauri/src/main.rs`, `ui/bridge.js`
- **Description**: Fetch the fixed update host/path on a blocking worker with finite WinHTTP timeouts and size limits; parse the manifest; verify SHA-256 and Ed25519 signature with the embedded public key; reject incompatible schemas or minimum app versions.
- **Dependencies**: Sprint 1 signing key and schema.
- **Acceptance Criteria**:
  - No URL supplied by the server is followed.
  - Only strictly newer catalog versions are accepted.
  - Network and parsing never run on the Tauri event loop.
- **Validation**: Rust tests for tag/version comparison, hash/signature, limits, timeout/error mapping, replay, and schema rejection.

### Task 3.2: Add crash-safe last-known-good catalog caching

- **Location**: `src-tauri/src/catalog.rs`
- **Description**: Keep current, previous, and pending catalog files outside user-authored settings; use write-through atomic replacement and quarantine corrupt files.
- **Dependencies**: Task 3.1.
- **Acceptance Criteria**:
  - Corrupt current data recovers from previous.
  - Both invalid caches fall back to the bundled catalog.
  - A network failure never removes a valid cache.
- **Validation**: Filesystem recovery and interrupted-write tests.

### Task 3.3: Merge and apply the effective catalog

- **Location**: `ui/index.html`
- **Description**: Merge bundled catalog, verified remote snapshot, and local custom stratagems by stable ID before restoring loadouts; rebind active slots after an in-session update; invalidate OCR cache; synchronize the native shortcut filter, main list, and overlay.
- **Dependencies**: Tasks 3.1–3.2.
- **Acceptance Criteria**:
  - Local `custom_` entries cannot be overwritten remotely.
  - Disabled remote items remain functional in existing loadouts but are hidden from the library.
  - Existing loadouts and presets continue resolving by ID.
- **Validation**: Focused UI/OCR/native-filter regressions and a real Tauri/WebView2 check.

### Task 3.4: Add the catalog update notification and diagnostics

- **Location**: `ui/index.html`, `src-tauri/src/diagnostics.rs`, `src-tauri/src/runtime_diagnostics.rs`
- **Description**: Show a concise “stratagem catalog updated” dialog with version and item count on the next startup; report current/previous/bundled source, last success, and compact failure code in Diagnostics Center.
- **Dependencies**: Task 3.3.
- **Acceptance Criteria**:
  - Notification is shown once per applied version.
  - Failure is diagnostic-only and never blocks normal use.
- **Validation**: Persistence and presentation regressions plus real installed-app verification.

## Sprint 4: Production Deployment and End-to-End Acceptance

**Goal**: Deploy without disturbing existing server workloads and prove the complete public workflow.

**Demo/Validation**:

- Cloudflare login protects only `/admin/*` and `/admin/api/*`.
- Public manifest/catalog remain anonymously readable by clients.
- Publish through the GUI, restart an installed client, and observe the update notification and new stratagem.

### Task 4.1: Package an isolated Linux service

- **Location**: `server/stratagem-admin/deploy/stratagem-admin.service`, `deploy.sh`
- **Description**: Install under `/opt/hd2-stratagem-admin/releases/<timestamp>`, retain a previous release symlink, run as a dedicated user on `127.0.0.1:8785`, and keep persistent data under `/var/lib/hd2-stratagem-admin`.
- **Dependencies**: Sprints 1–2.
- **Acceptance Criteria**:
  - No existing service, port, file ownership, or domain changes outside the scoped Nginx site.
  - Restart, failure, and rollback are bounded and logged.
- **Validation**: `systemd-analyze verify`, service health, journal warnings, process user, and listener check.

### Task 4.2: Add scoped Nginx routes and Cloudflare Access

- **Location**: `/etc/nginx/sites-available/update.unsnow.online`, Cloudflare Zero Trust application
- **Description**: Proxy `/admin/` and catalog endpoints to the loopback service while preserving the existing Release proxy; configure Access specifically for `/admin/*`; obtain team domain/AUD and set them in the service environment.
- **Dependencies**: Task 4.1.
- **Acceptance Criteria**:
  - Existing `/api/v1/releases/latest` response remains byte/behavior compatible.
  - Direct-origin requests cannot forge admin access because the application verifies Access JWTs.
  - Nginx configuration passes before reload.
- **Validation**: Public and direct-origin probes, unauthenticated/admin browser tests, and existing update endpoint comparison.

### Task 4.3: Full client and server acceptance

- **Location**: Repository root and production service
- **Description**: Run all repository checks, bundle Windows x64, test clean install and upgrade, execute admin publish/disable/restore/rollback, and verify a real installed client.
- **Dependencies**: All previous tasks.
- **Acceptance Criteria**:
  - `npm run check`, `npm test`, and `npm run build:bundle` pass.
  - Clean and previous-release upgrade installs pass.
  - Public manifest, immutable catalog, signature, GUI publication, rollback, client notification, OCR, shortcuts, and overlay all pass end to end.
- **Validation**: Recorded command results, screenshots, artifact version/checksum, and server health snapshot.

## Testing Strategy

- Node unit tests for schema validation, image normalization, JWT validation, signing, atomic publication, concurrency, history, and rollback.
- HTTP integration tests with temporary data roots and generated keys/tokens.
- Existing UI, OCR matcher, Rust unit, formatting, and Clippy suites.
- New Rust tests for manifest and catalog cryptography, compatibility, size limits, cache recovery, and downgrade protection.
- Real Tauri/WebView2 validation and real browser admin validation at desktop and mobile widths.
- Clean install and upgrade over the previous public release before any client release.
- Production probes for public domain, direct origin, health, existing release proxy, Access denial, and authenticated admin mutations.

## Potential Risks & Gotchas

- Cloudflare Access protects browser traffic, but the origin must still validate the Access JWT because the server IP is public.
- Immediate publishing means every Save is production; use a concise confirmation, optimistic concurrency, complete validation, immutable history, and one-click rollback.
- Arbitrary SVG is active content; rasterize every upload before wrapping it in the normalized SVG container.
- A signing key stored on the publisher host can be stolen if the host is fully compromised. Restrict the key to the service account, keep an encrypted offline backup, document rotation, and consider a managed KMS later.
- Never hard-delete IDs referenced by user loadouts. Disable them and keep them resolvable.
- Do not block startup on network access; use bundled and last-known-good catalogs first.
- Cloudflare configuration cannot be completed without an authenticated dashboard/API session and the Access audience/team-domain values.

## Rollback Plan

- Catalog content rollback: publish a new higher version copied from any historical snapshot.
- Service rollback: move `/opt/hd2-stratagem-admin/current` to the retained previous release and restart the isolated unit.
- Nginx rollback: restore the timestamped site backup, run `nginx -t`, then reload.
- Client rollback: ignore or remove the remote cache and fall back to the bundled catalog; revert the isolated feature commit if necessary.
- Cloudflare rollback: disable or remove only the `/admin/*` Access application without changing public catalog routes.
