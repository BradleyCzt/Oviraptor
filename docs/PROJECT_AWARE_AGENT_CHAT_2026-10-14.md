# Project-aware Agent chat

Status: implementation in progress; not included in the earlier `18ba865` preview.

## Product contract

- A fresh application launch opens Chat, even when the previous route was a plugin. Project selection and history remain persistent.
- Conversation history belongs immediately below Chat in the left navigation. Built-in plugin entries occupy the bottom section directly above workspace selection.
- Assets and Scanning remain independently usable modules. Opening Chat does not mount or initialize their workspaces.
- Oviraptor is the application name. The model receives the actual selected project identity from trusted application state; project names/descriptions are data, not instructions.
- The model may select relevant enabled tools. The host validates scope, arguments, limits, cancellation and grants before executing any tool.
- Only application-relevant tools are included. The initial adapter supports project context, scoped asset and scan-result reads, approved Skills and approved directory text reads. It does not grant arbitrary shell, browser, network or cross-project access.
- A real scan remains subject to the existing explicit target/start flow. Reading scan results must never claim to start a scan.
- Permission controls explain that selected tool data may be sent to the configured model. Changing a grant does not replay an interrupted request.
- Automatic titles use the existing model interaction. Manually supplied/edited titles remain protected.
- Tool activity is attached to the correct reply and displayed immediately within that turn, not as a distant global panel.
- Steer remains next-reply direction; it cannot retroactively inject into an in-flight provider request.

## Required verification

1. Fresh launch from persisted Assets/Scanning route opens Chat, preserves project, and does not issue plugin initialization calls.
2. Expanded/collapsed sidebar has one history and one plugin group in the agreed order; dark/light and narrow layouts remain usable.
3. Two projects with different names/data remain isolated in model context and tools. Model-supplied project IDs cannot override host scope.
4. Enabled tools run through real gateway tool-call round trips using synthetic transport tests. Plain-text answers remain supported.
5. Unknown/disabled/malformed calls fail visibly; step, output and deadline limits stop loops; cancellation and uncertain transport do not replay requests.
6. Tool activity persists across reload and remains attached to its original conversation/reply.
7. Grants are explicit, revocable and scope-bound. Path traversal, symlink escape, sensitive credential paths, binary/oversized reads and stale grants are rejected.
8. Skills are read as untrusted guidance and do not expand permissions or execute commands.
9. AI title persistence respects manual edits, old-title provenance, reply completion and concurrent revisions.
10. Steer, archive/delete, context pagination, uncertain send recovery, no-model save-only mode and plugin direct use remain functional.

## Evidence and release status

Add exact test results and compiled source identity only after verification. Browser fixtures are synthetic and do not demonstrate real provider or target execution. Do not label this version shipped until a verified package exists.

## Additional Skills request

The user also requested the 36-skill Every Compound Engineering package and a standalone local custom-package import entry. Official upstream is https://github.com/EveryInc/compound-engineering-plugin at `50532b00de523980010d97fe950965b35382a3b7` (MIT). The downloaded archive was compared with the pinned Git tree: 1,488 regular-file Git blob hashes match. This verifies source identity, not behavioral safety or tool compatibility.

Import must retain Skill instructions, referenced resources, license and provenance without executing bundled scripts. Package availability is distinct from the host's executable capabilities. Git operations, browser execution and Xcode workflows must not be advertised as operational merely because their Skill was imported. Existing custom skills must not be silently overwritten. Conversation grants determine which installed skills may be read by the model.

## Checkpoint: 2026-10-14 02:12 UTC (NOT A RELEASE)

Implementation was interrupted by the development execution preview limit. This checkpoint is intentionally separate from the last usable preview and must not be installed or merged as a completed feature.

Verified so far:
- Frontend production TypeScript/Vue/Vite build passes.
- Application-shell tests pass: 107/107; additional access UI tests pass: 16/16, including new startup routing, persistent history, worker-sync gating, inactive chat/Steer and per-turn activity cases.
- Earlier focused navigation checks passed; automatic-title SQL guards were exercised separately (17 SQLite checks), but those do not replace Rust tests.
- Official bundled data contains 36 Skills / 443 files with exact source and license preservation.

Implemented but not yet accepted:
- Chat-first startup, bottom plugin grouping, standalone lazy Skills view and local package preview/import UI.
- Real selected-project model context, bounded model-tool loop, strict tool-call parser, scoped asset/scan reads, persisted activity and AI title provenance.
- Basic per-conversation access controls and no-follow directory reads.
- Declarative package storage/parser/importer and resource browsing.

Known unfinished blockers at interruption:
1. `conversations/access.rs::validate_scope_revision` still rejects `agent_enabled=false`; plain chat must validate scope without requiring agent mode.
2. Provider disclosure is currently a boolean. Bind optional tool grants to the selected provider identity, show a sanitized recipient label, and require renewed acknowledgement after provider changes.
3. In-flight selected-Skill fingerprints/enabled state are not yet fully revalidated; revoke or edited instructions must prevent later sends using stale consent.
4. Access still limits selections to 24 and uses a 16KiB monolithic Skill read. Support all 36 selected Skills and bounded paged instructions/resources. Wire `skill_packages::list_resources/read_resource` into model tools; do not treat visible package resources as automatically granted.
5. Directory read/tool result byte limits need alignment (16KiB read vs 12KiB overall tool result).
6. `skill_packages/mod.rs` declares a test module, but `skill_packages/tests.rs` was not completed. `conversations/access.rs` also declares `access/tests.rs`, which is missing. Add real importer/bundle/collision/provenance/traversal/resource tests, including lazy-install collision handling.
7. Native Rust compilation/tests have NOT run on this checkpoint. Run conversation, strict protocol and Skill package tests on macOS CI and fix all compile/runtime failures.
8. The newly added permissions/Skills browser scenarios and fixtures need completion and actual Chromium validation; passing previous preview scenarios is not evidence for these changes.
9. Full UI/asset regressions, independent final review and ARM64 packaging/checksums are outstanding. No new downloadable build exists.

Continue from this checkpoint without changing the previous preview link. Keep permission changes explicit and avoid replacing unresolved capabilities with UI-only success messages.
