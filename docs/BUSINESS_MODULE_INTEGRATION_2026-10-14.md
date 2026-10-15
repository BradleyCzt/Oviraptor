> Historical first-round record. Current module ownership, second-round regression and local-handoff blocker: [2026-10-15 report](BUSINESS_MODULE_BOUNDARIES_2026-10-15.md).

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
and escalation did not make the transfer succeed. The final 2026-10-15
02:16 UTC retry again resolved metadata and size but returned `workspace_path=null`;
the official helper with explicit network permission exited 1 with download
failure. The archive was absent afterward. No archive bytes were
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
   pass at those recorded stages. The frozen final default-feature Rust
   library result and all-targets compilation are recorded below.
   The independent branch has been pushed; it has not been merged.

Exact WIP baseline was archived to `/workspace/baseline-d61454c` for independent
validation. Dependencies were installed from the official npm registry with
the existing lockfile. Rust was installed from `static.rust-lang.org` into a
private workspace toolchain; no operating-system install or application
dependency upgrade was performed. Completed command outcomes, retained failed attempts and final source
identities are recorded below and in the structured verification index.

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
  The initial complete run used four test threads; the final accepted run
  uses eight, as recorded below.

The completed frozen-source results and source identities are recorded below. This work continued into 2026-10-15 UTC.

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
review approved this boundary. In `bc65ad1`, Source fixture hashes and production financial/lease
semantics were not changed to accommodate the Web validator.
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
identified in `rust-final-full-input-manifest.json`; later source edits do not change that already loaded binary. The old run remains separate from final
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

### Remaining full-suite corrections

`2879d0e` repairs an actual Mapper frozen-request incompatibility: the factory
writes ten fields including `providerOptions`, while the old reader only
accepted the legacy nine-field shape. The reader now accepts the exact original
nine-field history without mutating or rehashing it, or the current ten-field
shape whose options exactly match the frozen model and 512-token output cap.
A shared pure derivation preserves the original GatewayProfile behavior,
including the three DeepSeek aliases. Current scheduling still requires the
full current frozen wire; a history-readable nine-field task cannot silently
become a new ten-field live request. Unknown fields, changed options and
configuration drift remain rejected. The two-request/8,000-token Mapper budget,
512-token output limit and one lookup are unchanged.

`3cb85fb` repairs legacy Native closure when an authenticated original birth has
`bootstrapDispatch=None`. Only after original fact/declaration equality,
Multi-mode and birth verification does this mean “no HTTP cost advisory”. It
does not authorize HTTP execution. Actual transport still refuses that legacy
executor at `http_budget_receipt_original_v8_protocol_required`, before claims,
fees, receipts or network. Cold protocol deletion and forged births remain
errors. `e7c0ee0` restores the saved-Mapper test's actual original v2 birth rather
than migrating its fixture to hide this production incompatibility.

Legacy complete finish reentry may refresh exactly `lease_expires_at`,
`heartbeat_at` and `updated_at` on its unique original Coordinator lease. Its
row identity, owner, epoch, fence, all other fields/lease rows and all business,
financial and permission facts remain unchanged; there are no new SDK/target
calls. This existing legacy heartbeat behavior is not a whole-database
zero-write guarantee. Modern supervision/cost-advice replay retains the
stronger complete database snapshot/zero-write assertion. The legacy exception
is not applied to modern replay or HTTP admission negatives.

`3c4f5cd` repairs the original unbounded `tokens=0, requests=0` grant. Such a
verified original grant legitimately has no reserve row. Only that exact pair
uses the existing optional frozen-row capture to freeze the precise empty set;
finite and mixed grants still require reserve rows and reject absent, changed,
forged or duplicate facts. Zero is the existing unlimited sentinel, not a new
free execution permission. Budgets, charges and authorization are unchanged.

The remaining test-only repairs preserve original accounting and admission:
`6891ef0`, `a6ed75f`, `f0cb0be`, `d9cf156`, `4103b3d` classify exact paid budget
advisory phases and their received receipts/publications/four-dimensional fees,
separately from the unchanged business calls. They retain bounded no-grant
advice and free replay checks. `1c50272` enables the actual financial publication
hooks for one real-completion fixture. `dbfd52f` fixes the test snapshot of an
existing `WITHOUT ROWID` table by preserving its composite primary key, while
rowid tables retain row identity. `fd29e4c` and `1d9d792` assert all 18 actually
wired typed triggers and distinguish not-ready/human-recovery cases.

`9029228` and `13c4855` permit only absent `scripts`/`public` source roots. When
present, both are recursively audited, including arbitrary extensions; root,
nested and dangling symlinks still fail, and required roots cannot disappear.
Generated QA dependencies were moved outside audited source without changing
the dependency tree. Each of thirteen existing retirement records was compared
against its original reviewed source and all changed literal contexts.
`c5f68e0` refreshes only their hashes/reasons/dates: no entries or categories were
added/deleted, no source scope changed, and the empty active baseline remains.

The first accepted all-targets compilation at `c5f68e0` failed with E0277:
one test helper inferred SQL COUNT as `usize`, which has no rusqlite FromSql.
`35a900a` reads `i64`, retains the exact-one assertion and uses checked conversion.
This two-line test-only correction has its own commit. The initial 97.61-second
exit-101 log is retained; the corrected all-targets check passed in 118.88 seconds.
New execution results must be filled from their completed logs, not inferred
from these source changes.

### Retained adapter boundaries and the next batch

The source dependency evidence is based on actual TypeScript/Vue AST imports:
196 source files and 189 cross-owner file import/export edges, including type
imports, lazy imports, CSS and assets. It is not a count of unique business
module dependencies or runtime authorities. `frontend-dependencies.json`
retains every path and import kind.

Worker settings still compose node configuration, environment reports and
remote scan listing/control/history synchronization. The two `runtime` imports
of `SentinelScan` are type-only, but express real Scanning ownership; this
management adapter must not be described as a pure transport layer. Environment
installation APIs/log DTOs still live in `runtimeApi`, which the Environment
panel consumes. `useCommittedRefresh` is truly shared by Assets and Scanning:
its callers supply business scope/payload checks, and the implementation owns
only event reconciliation and lifecycle. It is platform infrastructure, not a
platform-independent utility. Root i18n/CSS files still mix application-wide
presentation with business words/selectors and are not all generic shared code.

The next incremental cuts are: move environment APIs/log DTOs to Environment
behind explicit compatibility forwarding; move remote scan summaries/control
behind a Scanning adapter while keeping Worker identity/transport in Runtime;
shrink API facades after their real callers migrate; and move business CSS and
translations in batches that preserve the current cascade/theme behavior.
Keep Scan DTOs with Scanning instead of moving them into shared to remove an
import arrow. The scan/Agent native include scope remains a separate coherent
migration requiring its admission, financial, lease and replay tests together.

`6e5da74` scopes two existing successful exclusive-publication tests to macOS,
where the original production publisher uses the held-directory exclusive
rename primitive. That production file and writer are unchanged from `d61454c`;
non-macOS explicitly returns unsupported instead of falling back to an unsafe
rename. Two Unix/non-macOS tests exercise actual first publication and immutable
existing-decision cache reconstruction. Repeated refusal must preserve all raw
DB rows/identities and total changes, the exact still-valid preparation lease,
all configuration identities/bytes and absent cache; source reads must retain
the precise missing-decision/missing-source refusal. Cleanup follows assertions.
These Linux controls do not count the two Mac positive tests as passing. On
Windows the pre-existing permission gate rejects earlier; neither publisher
path is claimed to be validated there.

The first fresh 6e5da74 fast selection completed 371 passed / 1 failed / 0 ignored
(out of 372 selected) in 121.67 seconds; it did not authorize a full-suite pass.
Only the opted-in new-evidence fixture still expected two cost assessments,
although its original trusted finite grant, received Web bills and settled
usage each contain three model requests. `496b98f` strengthens this test instead
of merely incrementing a counter: all three distinct original calls must match
their round, request/response hash, usage and dispatch, their canonical Web fee
source and all four cost dimensions. Each paid Root assessment must match its
received invoice, published usage and exact four-dimensional consumption; the
single HTTP advisory must match its sealed receipt and the original reserve,
forfeit and reconcile entries. Exact no-grant/assess-only publications and all
bootstrap/allocation/Mapper/business counts remain asserted. The unchanged
main case still requires two local assessment model calls, two original charges
and complete database/SDK/target zero-increment final replay. Other deliberately
unpaid fixtures remain unchanged. The follow-up runs the whole neighboring
new-evidence group before the complete suite; the old 371 passes are not
combined with it to claim a 372-case final-source pass.

### Historical complete-run failure inventory

The original complete loaded binary finished naturally: 3,501 passed, 77 failed, 7 ignored, no filtering, in 4032.90s. The subreaper reaped 1,715 orphan descendants. Its recovered process session
did not retain an outer exit-code handle; the Rust FAILED summary is complete
and retained without inventing an outer exit status. The following inventory
matches all 77 unique FAILED names exactly; classification is not itself a pass.

| Historical failures | Classification |
| ---: | --- |
| 2 | `authenticated_http_fixture` |
| 7 | `coordinator_contract_fixture` |
| 1 | `legacy_none_closure_and_original_mapper_recovery` |
| 2 | `paid_resume_and_fresh_round_contract` |
| 2 | `revocation_financial_advice` |
| 1 | `wired_inventory` |
| 3 | `lease_financial_advice` |
| 19 | `mapper_provider_options_contract` |
| 15 | `diagnostic_test_snapshot_without_rowid` |
| 1 | `original_financial_publication_before_closure` |
| 1 | `ordered_execution_financial_advice` |
| 8 | `protection_financial_phase_or_earlier_binding_refusal` |
| 2 | `optional_retirement_source_manifest` |
| 8 | `legacy_none_closure` |
| 1 | `exact_wired_trigger_inventory` |
| 2 | `verified_zero_grant_reserve_snapshot_and_financial_advice` |
| 2 | `macos_exclusive_publication_platform_contract` |

Per-test commits and the two original Mac-positive to Linux-refusal mappings
are retained in `validation/rust-broad-failures-progress.json`. The two Mac
positive bodies remain in source and are not counted as Linux passes.
Of the 77 historical failures, 73 original test names pass in the final run;
two paid-resume tests now have explicit refusal/fresh-root names and changed
contract assertions. Those two passing replacements do not implement or
prove the original planned continuation behavior. The final two are the
Mac-only positives, with separately named Linux refusal controls passed.

### Frozen final acceptance — 2026-10-15 UTC

Accepted source: `496b98f7dee6fcd29bc275e488b539c5705c6180`; the worktree was clean before execution.
Native compiler inputs, the actual recursively audited path set, all tracked
inputs and the loaded binary SHA-256 were captured before execution and checked
after it. Native/audit path additions, removals and byte changes are failures;
only explicitly listed final documentation changes are permitted.

| Check | Completed result | Archive log |
| --- | --- | --- |
| Vue/TypeScript and Vite production build | Passed; frontend build/browser inputs remain identical | `accepted-build.log`, `browser/current-source-verification.json` |
| Complete Node glob | 1,512 passed, 12 platform skips, 0 failed; tracked inputs unchanged during execution | `final-accepted-all-node.log`, `final-accepted-node-input-manifest.json` |
| Default-feature all-targets compilation | Exit 0, 19.03s | `rust-accepted-all-targets-final.log` |
| Default-feature library test build | Exit 0; actual Cargo artifact/profile recorded | `rust-accepted-test-no-run.log`, `rust-accepted-test-no-run-result.json` |
| Fast targeted native regressions | 25 passed, 0 failed; explicitly filtered, not a full suite | `rust-accepted-targeted.log`, `rust-accepted-targeted-policy.json` |
| Complete default-feature library suite | 3,593 passed, 7 ignored, 0 failed, 0 filtered; 1532.34s | `rust-accepted-full.log`, `rust-accepted-full-result.json` |
| Actual Chromium UI | 18 passed; no runtime errors, unknown IPC or external fetches; synthetic IPC | `browser/browser-results.json` |

The Node command is:

```sh
CHROME_BINARY=/usr/bin/chromium OVIRAPTOR_TEST_NO_SANDBOX=1 \
  node --test --test-concurrency=1 tools/test*.cjs
```

Native acceptance uses a separate target directory, test optimization level 1,
debug information 0, and explicit `debug_assertions=true` and
`overflow_checks=true`, with two build jobs and eight full-suite test threads.
Repository profiles and dependency lockfiles are unchanged. All six real
600-second natural lease-expiry cases execute unchanged in the complete suite;
the two repaired Mapper expiry cases are deferred from the fast selection to
that full run, avoiding a duplicate ten-minute wait. Exact commands and
environment are in the structured result files; the complete run command is:

```sh
python3 tools/run_with_subreaper.py cargo test --manifest-path src-tauri/Cargo.toml --locked --lib -- --test-threads=8
```

The seven existing ignored native cases are two live external-read cases,
four explicit scale tiers and one extracted-project-copy acceptance case.
They were not counted as passes or force-enabled. Linux does not compile/run
the Darwin-only zombie-group case or the two Mac exclusive-publication
success cases. The optional `import-tools` feature, packaged Tauri GUI/real IPC,
Mac/ARM64 installers, real model providers/scan targets and user databases
are outside this verification. Browser screenshots remain local and are
excluded from Git and the evidence archive.

[The structured verification index](integration/verification-2026-10-15.json)
contains actual outcomes, commands, source identities, per-commit file
inventory and remaining limits. The accompanying
`Oviraptor-business-modules-regression-20261015.tar.gz` contains the full
text/JSON logs, manifests, failure attempts and SHA256SUMS. Documentation is
committed after the accepted code source; its hash is reported separately
with the final pushed branch. No merge to main is performed.

The local handoff remains unmaterialized and unintegrated. Restoring the
cloud Library byte-transfer path is required to complete the requested
three-source semantic integration. Cloud regression success does not remove
that blocker or implement the pre-existing paid financial continuation gap.
