# Internal execution service: compatibility-first extraction

Baseline: `94cdb8a`. This is one internal module boundary, not a new product surface,
new sandbox platform, generic execution authorization, or completed headless runtime.

## Scope and ownership

- `execution_service/host_process.rs` owns the existing bounded host-process implementation:
  argv forwarding, optional cwd, process-group cleanup, deadlines, cancellation, captured
  pipes, output bounds and observer dispatch
- `execution_service/process_output.rs` owns the existing bounded capture/record pump
- `execution_service::OutputEvent` is only a redacted diagnostic record: stream, stream
  sequence, message and gap. It does not carry or grant execution authority
- `native_pipeline/process.rs` and `native_pipeline/process_output.rs` preserve existing
  import paths as compatibility adapters. Existing Native helper callers still use the
  same `OwnedChild`, pipe and output implementation
- `native_pipeline/process_log` continues to own scan/attempt/branch/dispatch/execution
  scope, persistence, append guards, replay and persist-before-notify behavior. `Event`
  is a type re-export, with no lossy translation or persisted schema change
- `execution_service/offline_parser.rs` adapts only the existing production guardian
  dispatch and existing test-runtime dispatch. `homogeneous/parser_execution.rs` retains
  all request freezing, original paid-response binding, approval and worker checks,
  periodic cancellation/revalidation, reconciliation and signed result persistence

The implementation contains no Tauri command or top-level UI. Project Chat, Assets and
Scanning may eventually use application adapters above this internal service. Adding a
consumer does not authorize it to execute arbitrary code or reuse another consumer's
lease. There is no shared policy default or implicit project/run/attempt inference here.

## Guarantees that remain different

### Bounded host command

The host runner inherits its environment, accepts an optional cwd and runs the exact
caller-supplied argv. An absent cwd continues to use the current directory. It bounds
capture and lifetime and retains existing process-group cleanup. These properties do
not establish filesystem, network, memory, credential or complete descendant isolation.
A caller deliberately invoking a shell remains responsible for that choice; the runner
does not introduce a shell itself.

The deterministic host mechanism needs no LLM configuration. This does not remove the
existing domain admission requirements or turn every analyzer into a host-enabled tool.
In particular, the existing production analyzer configuration still requires its pinned
container image where its caller policy says so.

### Approved offline parser

The fixed container implementation still requires the existing macOS runtime inspection,
Docker identity, fixed image, cgroup/seccomp policy, no network, constrained resources,
receipt checks and guardian cleanup. The original host/Seatbelt parser remains unsupported
because hard per-run memory containment is unverified. Non-macOS container inspection
continues to return `unsupported_sandbox:container_parser_platform_unverified`.

The adapter does not select another runtime, supply an image, pull a package, broaden
filesystem/network policy, approve a package, issue a lease, or fall back to the host
runner. The existing homogeneous production caller is still coupled to its original
paid model response. This change does not create an independent no-LLM parser capability.

`container_parser.rs` hashes its own source and `container_parser/orphans.rs` into the
administrator-approved runtime digest. Both files remain byte-identical to the baseline:

- `container_parser.rs`: `ab97783c54a02578a94225d08fc9a3425d31cf214808ff90169d172fde7dd3dd`
- `container_parser/orphans.rs`: `b7f2548e305522be915b57ac20179e53cb5ce2e18d39bddd27961f577db6803c`

Moving or editing those files later requires explicitly accounting for approval-digest
changes rather than describing it as an invisible reorganization.

## Existing context and lifecycle contracts

Project, run, scan, attempt, assignment, lease epoch and fencing authority stay with their
existing domain owners. They must not be replaced by a caller-supplied generic context
object or by a successful mechanism call. Host process cancellation remains the caller's
callback; offline parser cancellation remains the callback that also revalidates the
frozen invocation. Native invocation locks and durable dispatch/recovery receipts are
unchanged. An OS lock release is still not a retry permit.

The process journal continues to add scope before persisting and notifying. Task-state
transitions, outcome receipts and completion hints remain distinct from stdout/stderr
records. No new event schema, state machine, persistence writer or retry decision is
introduced by this extraction.

## Validation

Added tests under `execution_service::tests` cover:

1. Pre-cancellation and exhausted deadline prevent executable lookup/launch
2. The diagnostic record is exactly the legacy journal Event type
3. Non-macOS offline parser dispatch fails closed without a host fallback
4. Literal argv values and explicit/absent cwd retain behavior
5. Host environment inheritance remains unchanged
6. Legacy and service entry points retain raw byte caps and result semantics

Existing observer, redaction, overflow, replay isolation, cancellation, fork cleanup,
pipe inheritance and delayed EOF regression suites remain active. They are compiled
under `execution_service::host_process` using their existing test files. The one source-pin
assertion now reads the relocated host implementation; its required pipe properties are
unchanged. Existing Native analyzer/helper lifecycle and policy suites remain relevant.

Static checks performed during extraction: targeted rustfmt, `git diff --check`, original
versus current sandbox-source byte comparison, and review of the process/output relocation.
The implementation bodies differ only in the diagnostic type/module paths and formatting.
Coordinated Linux Rust verification compiled the new implementation and produced:

- `execution_service`: 22 passed, 1 failed. All six new compatibility tests passed
- `native_pipeline::tests`: 62 passed
- `native_helper`: 15 passed, 1 failed
- `execution_readiness`: 4 passed
- `tool_supply`: 51 passed, 2 failed

All four failures were reproduced using the preserved pre-change test binary. The
zombie-only process-group fixture assumes Darwin's `kill(-pgid, 0)` result but is enabled
on Linux; its assertion fails before invoking the sweep implementation. The Node selector
fixture expects a modern Node in its fixed system paths, unavailable in this environment.
Two approved-package tests require an exclusive-publish implementation unavailable on
Linux. These remain failed tests, not silently skipped passes. No cleanup or platform
policy was weakened to change their outcome. This was targeted coverage, not the full
Rust test suite or macOS sandbox acceptance.

## Deliberately remaining work and coverage gaps

- No Chat or Assets execution adapter has been implemented by this slice
- No generic project/run/attempt authority, task-event bus, scheduler or cancellation
  registry has been introduced; any future application adapter must preserve and test
  its own verified scope, admission, leases, receipts and persistence ordering
- Parser package approval, trust storage, frozen inventory and policy remain in
  `tool_supply`; its signed runtime implementation was not relocated
- Native analyzer container ownership remains in `native_pipeline/container.rs`; web
  helper orchestration remains in its existing command adapter
- Output redaction still depends on the existing `agent_runtime::secrets` utility;
  this is an in-crate boundary, not an independent Rust crate
- No new CLI, independent headless application, unsupported platform adapter or general
  code-execution API was added
- macOS fixed-container isolation and guardian/SIGKILL tests require the original real
  macOS/Docker environment. Linux compilation or unit tests cannot prove those guarantees
- Existing older-macOS pipe fallback and non-Unix process-cleanup limitations were not
  expanded, repaired or reclassified by this refactor
