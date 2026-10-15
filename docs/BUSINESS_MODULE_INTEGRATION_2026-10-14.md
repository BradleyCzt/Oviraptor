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
   [The migration map](integration/module-moves-2026-10-14.json) retains the 17
   component moves, two Skills moves and 128 declaration owners for that review.
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

Independent contract comparison against `d61454c` confirms that all 289
Tauri IPC registration names and their order are unchanged; only 33 owner paths
move. The configuration dialog is byte-equivalent after removing changed
imports, `db.rs` and `models.rs` are unchanged, and all six configuration-profile
functions retain equivalent tokens after visibility/helper normalization. All
128 original root type declarations match their nine destinations using the
TypeScript AST printer. Default Chat routing, both builtin module descriptors,
project props and the standalone Skills import control remain present. This
structural review supplements the executed regressions; it is not a claim of
live model/scan or packaged desktop acceptance.

## Project-aware Chat and Skill contracts

- Plain Chat scope/revision validation does not require `agent_enabled`. Without
  Agent mode it sends no tools; missing provider configuration does not create
  tool consent or block ordinary history/access operations.
- Consent binds the complete endpoint (including path/query), model and local
  deployment identity. Request construction and its permission lease are
  captured in one transaction; subsequent rounds and final reply commitment
  recheck the lease. UI labels expose only scheme/host/port.
- Selected Skill identity includes full content, package provenance and actual
  accessible-resource fingerprints. Persistent monotonic generations and
  deletion tombstones invalidate consent across disable/enable, edit/restore
  and delete/recreate, including equal timestamps and identical final content.
- The WIP already allowed 128 selected Skills. The final catalog can list all
  36 builtin Skills over bounded pages; this is not a promise that one reply
  can consume all their contents. Skill scripts remain inert text.
- Model instruction/resource pages use UTF-8 byte offsets, default/maximum
  4,096-byte bodies and explicit continuation. Catalog/resource listings use
  at most 12 items. The complete serialized tool envelope is bounded at 32 KiB;
  the request context is bounded at 64 KiB and further constrained by the model
  budget. Replies permit at most four model rounds, eight tool calls, four
  calls per round and a 90-second deadline.
- Directory reads validate the full bounded file before returning any page,
  including credential detection after the output boundary. Directory bodies
  are at most 4,096 bytes; full file inspection is limited to 1 MiB and there
  are at most eight directory grants. The standalone Skills browser has a
  4,096-byte default and 8,192-byte maximum page.
- Incremental migrations retain old directory/Skill selections. Old boolean
  consent does not silently become new recipient/content-bound consent;
  explicit acknowledgement is required again. Version-1 clients may revoke
  permissions, while enlargement without current acknowledgement is refused.

The formerly missing native modules now exist:
`conversations/access/tests.rs` (14 cases) and `skill_packages/tests.rs`
(16 cases). Focused native Chat tests pass 57/57 and package tests 16/16,
including the disabled-Agent case, endpoint changes during a paid reply,
same-timestamp Skill revocation, 36-Skill pagination, UTF-8 boundaries and
credential detection outside the first page.

## Batches and verification

1. Owner contracts: `e4a6199` moves 128 declarations into nine owner contracts,
   updates 100 consumers, and retains type-only compatibility exports.
   Frontend build and application-shell 107/107 passed for this stage.
2. Business components and Skills dependency direction: `9fd21c2` moves 17
   components to owners, extracts the shared instruction catalog into Skills,
   and guards actual static/dynamic imports with TypeScript/Vue AST traversal.
   See [frontend ownership](FRONTEND_MODULE_OWNERSHIP_2026-10-14.md).
3. Rust command namespaces: `50738a1`, `d9fe132`, `e3c5b7d` establish seven
   module owners and preserve 33 IPC names. Source guards and project deletion
   UI regressions pass 26/26; the extracted production inventory-frame parser
   passes its independent native Rust test 1/1. These do not prove whole-crate
   compilation. See [Rust boundaries](RUST_BUSINESS_MODULE_BOUNDARIES_2026-10-14.md).
4. Package source/integrity/import boundaries: `306d0fd`; scope/provider/Skill
   consent and paged resources: `b8636ae`; pending-consent revocation fix:
   `8247b74`; persistent revocation generations: `c480363`. Native conversations
   57/57 and Skill packages 16/16 passed in focused runs. Access plus real
   activation/race UI cases pass 27/27.
5. Browser QA: `1c4c3a7`, `227ff45`, `379b097`; actual Chromium acceptance
   passes 18/18. The complete frozen-source Node command passes 1,512 with
   12 platform skips. Production build and native production/test compilation
   pass; full Rust execution continues with failures recorded below.
   The independent branch has been pushed; it has not been merged.

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
| Same native test build after private system dependencies | Reached E0583 for both missing test modules in the checkpoint; stopped after diagnostics (exit 143), not passed | `/workspace/validation/rust-baseline-native.log` |

Dependency installation attempts are also retained: the first `npm ci` failed
because the default home cache was not writable; retry using the workspace
cache and official registry succeeded. Its incidental `yarn.lock` rewrite was
reverted before commits. The Library transfer attempts failed without creating
archive bytes. No failed or skipped check is represented as a passing test.

### Integration findings and failures retained

- Component migration initially failed type checking because three lazy-panel
  imports still used the old component paths. Corrected before the successful
  production build. The second UI run was 1,102 passed / 18 failed: 17 failures
  came from workspace harness API mocks at the wrong relative path; one came
  from the moved confirmation component mock. All 35 tests in the affected
  suites subsequently passed. Original failed logs remain in
  `/tmp/oviraptor-components-{build,ui,ui2}.log`.
- The first all-tools Node run was 1,517 tests: 1,371 passed, 134 failed and
  12 platform skips (`/workspace/validation/final-all-node.log`). This broader
  command includes tests absent from `test:ui`: an isolated exact-baseline run
  confirms all 128 human-review cases already failed because their API loader
  omitted the reconciliation module. Five workbench tool-selection failures
  were introduced by the new Skills catalog API dependency; the corresponding
  baseline tests pass 5/5. The remaining failure is the local CDP runtime
  fixture. None are suppressed or counted as passed; fixes and reruns follow.
- The first integration `cargo check --locked --lib` failed with E0425 in
  `commands/assets.rs`: a JSON helper had been inherited through `include!`.
  Commit `4c7b9f5` makes decoding explicit inside the asset owner and retains the
  invalid-JSON fallback. The fixed whole-crate production check passed with
  exit 0 in 33.85 seconds; log
  `/workspace/validation/rust-integration-check-fixed.log`.
- Independent review identified a request/consent capture race and pending
  consent accidentally blocking revocation; both were fixed in `b8636ae` and
  `8247b74`. `c480363` adds persistent monotonic Skill generations and deletion
  tombstones: disable/re-enable or edit/restore cannot revive old consent even
  when timestamps and final content match.
- Initial actual Chromium acceptance passed 17/17 with no runtime errors,
  unknown IPC or external requests. It used synthetic IPC and retained 29
  screenshots plus stable source manifests in `/workspace/validation/browser`.
  A later independent UI review reproduced stale consent labels when returning
  to the permanently mounted Chat page; `5c921a5` and `1504dcb` correct that
  navigation/configuration path and its stale response race. The final 18-case
  run covers the real return/save paths; the initial pass alone did not.

### Follow-up corrections and native execution

- `5c921a5` refreshes the existing Chat access panel on reactivation and after a
  successful configuration save. `1504dcb` prevents an old access-write response
  from replacing a newer recipient/Skill refresh. Seven real Workspace plus
  AccessPanel regression cases were added; they do not invoke a reload helper
  or remount the panel to simulate a working lifecycle.
- `b3d009c` makes human-review and workbench-selection fixtures load the actual
  API leaves through the existing harness. Actual corrected runs pass 128/128
  and 5/5, retaining the original behavior assertions. `d227d00` confines the
  loopback Chromium fixture's writable configuration/cache to temporary XDG
  paths. On this container only, the explicit
  `OVIRAPTOR_TEST_NO_SANDBOX=1` test launcher is required; production browser
  behavior and global `HOME` are unchanged. The corrected CDP test passes 1/1
  and proves anonymous capture, blocked cross-origin requests and comparison
  identity isolation against local synthetic servers.
- The complete native test target compiled successfully before execution
  (`rust-integration-test-no-run.log`, exit 0, 2m00s). Focused native runs pass:
  conversations 57/57, Skill packages 16/16, model protocol 34/34, and 45/45
  selected migrated command tests. Filtering is explicit in each log and is
  not reported as a complete Rust-suite pass.
- Initial `execution_service::` run was 20 passed / 3 failed. One existing
  test explicitly asserts Darwin's `EPERM` response for a zombie-only process
  group; Linux correctly returns 0. `ef35080` makes that platform-specific
  contract macOS-only, retaining all assertions. It still requires macOS
  execution and is not counted as passing on Linux.
- The other two failures retained killed descendants because this container's
  PID 1 is `tail` and does not reap orphans. A Linux child-subreaper control
  run passed both unchanged fork-storm tests (2/2) and reaped 994 orphaned
  descendants. `a798012` adds `tools/run_with_subreaper.py` as a reproducible
  outer test runner. It changes no application cleanup logic or test assertion.
  Complete native testing is run under this wrapper with four test threads.

Final all-suite results and source identities are recorded below only after
the frozen-source runs finish. This work continued into 2026-10-15 UTC.

### Wider native regression findings

The first complete-suite attempt was stopped after a known failing binary had
completed 590 tests: 577 passed, 13 failed, 0 ignored; 2,987 had not run.
`rust-full-suite-partial-summary.json` records this as incomplete. The 11 schema
fixture failures are addressed by `213e1ea`: the legacy-downgrade fixture removes
a trigger that could not exist in that legacy database and asserts it is rebuilt
on upgrade; the collaboration-event fixture selects only its own table DDL
instead of accidentally including later business triggers. Their original
production schema and fixtures were compared with `d61454c` Git blobs.

Two takeover tests and five HTTP-journal tests used pre-contract fixtures that
lacked authenticated Root birth records. `fce6274` supplies actual test-only v5
birth records while preserving the old fixture for malformed-history negative
tests. The first combined run was 28 passed / 5 failed; the remaining failures
exposed canonical target URL differences in the new fixture. `2023d9d` corrects
those targets, orders exposure crash-fixture identities by the real claim key,
and uses temporary Node version probes instead of assuming Mac installation
paths. HTTP-journal assertions remain unchanged. An unreachable-body server
also has bounded accept so an admission failure yields a failure instead of
hanging the test indefinitely.

Broader Source/human-directive tests exposed a separate production boundary:
Source plans have an owner-specific canonical hash that omits attempt identity,
whereas homogeneous Web execution inspected their raw JSON hash before deciding
whether it owned the plan. `bc65ad1` corrects the owner dispatch in
`homogeneous/{execution_policy,model_worker,material_policy}.rs` and
`native_pipeline/mod.rs`. Source ownership requires the immutable scope receipt,
strict canonical JSON, the Source hash, and matching row identity. Web policy
checks its original immutable receipt before inspecting mutable plan JSON, so
replacing and rehashing both Web plan representations cannot impersonate Source.
Historical Source reads do not depend on a live repository or current attempt.
Recognizing Source only closes Web capabilities and retains redaction; original
Source material validation remains mandatory. Eight added tests cover valid
owners, corruption, missing owner proof, historical reads, protocol replacement
and the distinction between ownership and execution authority. An independent
review approved this boundary. Source fixture hashes and production
financial/lease semantics were not changed to accommodate the Web validator.
The corrected production library and whole native test target compile; all
52 selected regression cases pass. Full-suite execution is recorded separately.

Actual final Chromium acceptance is now 18/18, with zero runtime errors, unknown
IPC and external fetches. The 225-file browser source manifest is unchanged
before build, after build and after browser execution. The final app bundle
includes `1504dcb`; QA commit is `379b097`. The earlier UI refresh failures and
late-response race are retained as real browser red-test evidence. Screenshots
remain local test evidence; they are not in Git or the Library evidence archive.

### Full-suite contract corrections

The next complete run of the compiled `cc45f2e` source exposed further old
fixtures and expectations. Its binary and 1,779 tracked native inputs are
identified in `rust-final-full-input-manifest.json`; later test-only edits do
not change that already loaded binary. The old run remains separate from final
acceptance.

- `aa87cb2` migrates two budget-transport positive cases and two fault-injection
  negative cases to authenticated real HTTP birth fixtures. Negative tests now
  require the precise claim/budget persistence error, retaining the original
  deadline, cost, rollback and zero-network assertions. The remaining 45 old
  fixture calls are documented individually in the call-site audit as direct
  claim/receipt/accounting, static artifact or deliberately invalid history.
- `03b280b` corrects two old paid-resume expectations. The production startup
  financial gate and related six files are unchanged from `d61454c`: paid
  history requires an explicit financial continuation contract that ordinary
  Resume/Retry does not implement. Real tests retain two provider calls and one
  target request before pause, then require both refusals to leave all database
  rows, total changes, work-file hashes and request counts unchanged. A separate
  real fresh Root retains inherited/skipped/zero-round admission checks.
  The existing unpaid-resume positive tests remain in place.

This paid-resume restriction is a pre-existing product limitation, not a new
successful continuation feature. It remains visible to the caller and must be
considered before using scanning pause/resume in production. Implementing the
missing financial continuation contract is outside this module migration.
