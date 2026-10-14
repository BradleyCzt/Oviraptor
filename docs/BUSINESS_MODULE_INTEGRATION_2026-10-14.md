# Business module integration — 2026-10-14

This is an integration and verification record, not a release announcement.

## Source identities

- Repository: `swyiic/Oviraptor`, cloud workspace `/workspace/Oviraptor`.
- Initial cloud checkout: `work` at `94cdb8a47b2ec4164c6e6c7b8297835fa579fcf4`.
- Preserved WIP baseline: `wip/project-agent-skills-20261014` at
  `d61454cdf73c52d190d3d4a26d6733356a6495af`.
- Integration branch: `integration/business-modules-20261014`, created directly
  from that WIP; `main` is not the refactoring baseline or merge destination.
- Original local source base: `8fddee2321b313a8a6372d0895dd4c1f533ba57c`.
  This commit exists in the cloud history and is an ancestor of the WIP.
- Local handoff: Library `libfile_4cfcbe4d240c8191b996a45dda2b97b8`,
  `Oviraptor-local-source-8fddee2.tar.gz`, expected 10,844,046 bytes and SHA-256
  `fc6448afee2399a988679ff64db8d95a2c5b7439e78ecd0bed05d94535fa24af`.
- Filtered bundle root: `0d0f995a2b7312c28589c1d952ae33ffcbe045db`.
  It is not the original Git common ancestor and must not be blindly merged.

No applicable `AGENTS.md` was present in the repository or its ancestors.
The project-aware Chat design, modularization audit, builtin-plugin design,
and repository architecture document were read before implementation.
No user Mac, local source directory, user database, model endpoint, or scan
target was accessed.

## Handoff blocker and remaining integration procedure

The Library materialization operation resolved the requested file, version,
filename and expected size. The current official transfer helper was obtained
and invoked in the cloud workspace. The cloud egress proxy rejected the signed
storage host with `403 Forbidden` during CONNECT. Explicit network permission
and escalation did not make the transfer succeed. No archive bytes were
obtained; its checksum, restore script, manifest, exclusions and local changes
have therefore **not** been inspected, restored or integrated. Library lookup
is available; byte transfer through this environment is the missing capability.

When transfer becomes available:

1. Materialize with Library metadata into a separate handoff directory and
   verify size and SHA-256 before extraction.
2. Inspect archive paths, the restore script, manifest and `excluded-files.json`
   before running the restore script into a new directory.
3. Preserve the filtered bundle as a source reference; compare original-base
   blobs at `8fddee2`, restored local edits, and the preserved WIP/current
   integration tree. Replay semantic changes in reviewable source commits.
4. Do not import unrelated filtered history as a common ancestor, overwrite
   cloud WIP, request secrets, or substitute main's configuration defaults.
5. The package omits `agent_runtime/tests.rs`, `commands/exposure/tests_g1.rs`,
   `commands/runner_log_write_tests.rs` and five historical fixtures. Preserve
   safe cloud tests where appropriate and create synthetic replacements only
   after checking the actual exclusion manifest.

The independent WIP improvements below do not imply local handoff integration.

## Ownership and dependency direction

| Owner | Frontend | Native boundary | Responsibility |
| --- | --- | --- | --- |
| Application shell | `src/shell`, `src/plugins`, `App.vue` composition | `lib.rs`, application settings/overview adapters | Chat-first routing, trusted builtin workspace composition, themes and global notifications |
| Project space | `features/workspaces` | `commands::workspace_projects` | Project identity, lifecycle and deletion guards; read-only impact aggregation |
| Assets | `features/assets` | `commands::assets`, `asset_export`, ownership/exposure modules and asset jobs | Inventory, ownership, exposure, collection, export and explicit recovery |
| Scanning | `features/sentinel` (stable internal name) | Existing scan commands, runtime, native pipeline | Web/Code/Greybox/CI, investigation timelines, execution, evidence and recovery |
| Chat | `features/conversations` | `conversations` | Independent project-bound history, replies, scoped read tools and grants |
| Skills | `features/skills` | `skill_packages`, existing skill catalog storage | Declarative instructions, package provenance/import/resources and explicit selection; no execution authority |
| HackerOne | `features/hackerone` | Existing HackerOne adapter | Compatible external-source builtin workspace |
| Configuration | `features/configuration` | `commands::workspace_config_profiles` | Full existing configuration round trips and explicit provider selection |
| Infrastructure | `features/runtime`, `features/environment` | Existing model gateway, worker, execution/tool-supply services | Worker and environment management; sandbox remains an internal capability |
| Shared primitives | `src/shared/components` | Explicit small infrastructure interfaces | Reusable modal/confirmation primitives only; no asset, scan, chat or skill business implementation |

The shell composes applications. Applications consume owner-specific contracts
and infrastructure APIs. Standalone Skills must not import scanning workbench
implementation. Scanning consumes the Skills catalog and retains its own task
selection adapter. Scanning investigation messages remain in Scanning; they are
not migrated into ordinary Chat. `src/types.ts` is a compatibility export
surface, not a new domain owner. Existing `src/api.ts` compatibility does not
grant a universal invocation or execution capability.

Rust migration is incremental: extracting a real module requires explicit
imports and visibility, Tauri registration at that module path, and preserving
the original command names and data contracts. Remaining scan/Agent `include!`
groups are documented technical debt; they are not rewritten as a generic
Agent or claimed to be isolated by directory changes alone.

## Batches and verification

1. Owner contracts: `e4a6199` moves 128 declarations into nine owner contracts,
   updates 100 consumers, and retains type-only compatibility exports.
   Frontend build and application-shell 107/107 passed for this stage.
2. Business components and Skills dependency direction: in progress.
3. Rust command namespaces and IPC ownership: in progress.
4. Scope/provider/skill consent and bounded resource tools: in progress.
5. Final regression, independent review and branch publication: pending.

Exact WIP baseline was archived to `/workspace/baseline-d61454c` for independent
validation. Dependencies were installed from the official npm registry with
the existing lockfile. Rust was installed from `static.rust-lang.org` into a
private workspace toolchain; no operating-system install or application
dependency upgrade was performed. Full command outcomes, failures and final
source identities will be recorded after the corresponding runs finish.

The design checkpoint's numerical limits are stale: source at `d61454c`
already permits 128 selected skills and a 24 KiB serialized tool result.
These values alone do not prove pagination, revocation, content fingerprints
or provider binding are correct.

### Exact WIP baseline results

All runs below use the independent `d61454c` archive, not the changing
integration working tree. Counts overlap between suites and must not be added.

| Command | Actual result | Cloud log |
| --- | --- | --- |
| `npm run build` | Passed Vue/TypeScript and Vite production build | `/workspace/validation/baseline-build.log` |
| `npm run test:application-shell` | 107 passed, 0 failed | `/workspace/validation/baseline-shell.log` |
| `node --test tools/test_conversation_access.cjs` | 16 passed, 0 failed | `/workspace/validation/baseline-access.log` |
| `npm run test:ui` | 1,120 passed, 0 failed | `/workspace/validation/baseline-ui.log` |
| `npm run test:exposure` | 118 passed, 0 failed | `/workspace/validation/baseline-exposure.log` |
| `npm run test:plugins` | 66 passed, 0 failed | `/workspace/validation/baseline-plugins.log` |
| `npm run test:release-bundle` | 11 passed, 12 platform skips, 0 failed | `/workspace/validation/baseline-release-bundle.log` |
| `cargo test --manifest-path src-tauri/Cargo.toml --locked --lib --no-run --jobs 2` before private system dependencies | Failed, exit 101: missing `glib-2.0.pc` | `/workspace/validation/rust-baseline-initial.log` |

Dependency installation attempts are also retained: the first `npm ci` failed
because the default home cache was not writable; retry using the workspace
cache and official registry succeeded. Its incidental `yarn.lock` rewrite was
reverted before commits. The Library transfer attempts failed without creating
archive bytes. No failed or skipped check is represented as a passing test.
