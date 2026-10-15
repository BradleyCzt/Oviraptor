# Rust business command boundaries

Baseline: `d61454cdf73c52d190d3d4a26d6733356a6495af` from
`wip/project-agent-skills-20261014`. The retained WIP is the behavioral baseline.
This migration changes command ownership and Rust namespaces while preserving
Tauri command names, input/output types and database behavior.

## Ownership and dependency direction

| Owner | Responsibilities | Allowed dependencies / boundary |
| --- | --- | --- |
| `commands::workspace_projects` | Project identity, archive, deletion admission and impact | Database and project DTOs. Cross-business record counts protect deletion; they do not initialize any business runtime. |
| `commands::assets` | Targets, asset inventory/filtering, decisions, recycle bin, asset-pipeline jobs/logs/events and interrupted-job recovery | Database, asset DTOs and `asset_logs` infrastructure. Scan status is a SQL read projection, with no scan execution dependency. |
| `commands::asset_export` | Asset CSV export | Explicitly imports the asset selection predicate, row mapper and SELECT projection from `assets`; export remains scoped identically to inventory. |
| `commands::application_overview` | Dashboard aggregate | Read-only shell projection across projects, assets and runs. No execution dependency. |
| `commands::application_settings` | Appearance, icon and reminder preferences | Database, file access and Tauri image decoding. No model or scanning dependency. |
| `commands::workspace_config_profiles` | Profile list/save/delete and preservation of inert stored fields | Database configuration policy. Does not resolve model credentials or execute tasks. |
| `commands::tool_administrator` | Signed inventory approval/revocation/read for administrators | Explicit `tool_supply::administrator_service` API and Tauri state. Task-external preparation metadata grants no execution permission. |
| `conversations` | Standalone project-aware chat, conversation scope, permissions, model tool loop and history | Separate existing module; does not initialize asset or scan workspaces. |
| `skill_packages` | Declarative Skill package provenance/import/resources | Separate existing module. Guidance/resources never add executable capabilities. |
| Legacy scan command group in `commands.rs` | Scan admission, control, native Agent orchestration, findings, evidence, ownership and exposure workflows | Still an explicitly retained migration debt. Existing internal modules such as `authorization_side`, `http_exchange` and exposure modules continue to enforce their narrower contracts. |
| Infrastructure outside `commands` | Database, logs/framing, execution service, worker transport, auth session and artifact import | Infrastructure owns mechanisms; a cross-business aggregate is not placed in a generic common module. |

`lib.rs` registers migrated commands using their actual module paths. Frontend
IPC strings are unchanged. Each migrated module has its own imports and cannot
implicitly access the private functions/imports of the legacy scan/Agent scope.
Ownership regression fixtures have one explicit test-only `assets::asset_filter`
dependency; CSV export has three explicit production asset query dependencies. No wildcard parent import restores
the old shared namespace.

## Migration limits and next cut

This is a gradual extraction, not a claim that all scan/Agent `include!` files
are isolated. Moving the entire scan family would require enumerating its
admission/authorization/budget/process dependencies and verifying their replay
and cancellation tests together. Ownership/exposure cannot be moved as arbitrary
individual files: they share transaction, authorization, source/result and review
contracts with scan execution. The next cut should place each coherent scan
business family behind explicit APIs and retain its existing native Agent rather
than rewrite it as a generic Agent.

`models.rs` remains the existing transport schema aggregation. No new shared
business layer or generic bucket was created. The separate frontend migration
owns its DTO extraction; later Rust DTO extraction must preserve serialization.

## Verification

- `node --test tools/test_rust_business_boundaries.cjs`: 8/8 passed. Guards real
  Rust module declarations, explicit dependencies, all 33 migrated IPC routes
  and the shared asset filter dependency.
- `rustc --edition 2021 --test` on the unchanged production
  `ToolInventoryFrame` parser and its existing `tool_inventory_frame_tests`
  source section: 1/1 passed, covering exact frame sizes, invalid magic, empty
  sections, truncation, trailing bytes and upper bounds. This isolated std-only
  check does not compile the surrounding Tauri commands.
- Final integrated source `496b98f` passed default-feature all-targets
  compilation and the complete library suite (3593 passed, 7 ignored).
  See the integration report for exact source/binary manifests, failed attempts,
  platform exclusions and commands. Source guards alone were not treated as compilation.
- Existing project/configuration/asset Rust tests retain narrow test-only
  imports of their moved helpers; the production command registration uses the
  actual owning modules.
