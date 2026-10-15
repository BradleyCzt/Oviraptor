# Evidence publication preparation

Current state: scripts and selection policy only. Application validation is still running. No final evidence archive has been created, no repository file was changed, and no Git commit or push was performed by this work.

The source is `/workspace/validation/asset-accuracy-20261015`. The eventual repository destination is `docs/asset-accuracy-20261015`; this script does not copy into the repository. The coordinator must first confirm validation has finished and provide the full frozen Git SHA (currently labeled `e5fed600`, subject to that final confirmation).

Selection includes every regular evidence file: historical failures and fixes, raw logs/stderr, exact command receipts, helper source, policy/environment records, before/after full input manifests, compiled/declaration/ignored-case ledgers, synthetic browser screenshots and local visual review records. It excludes retained native executables, generated Python bytecode caches, redundant paused-WIP source archives/bundles, and all symlinks/nonregular filesystem objects. The paired native executable retention proofs and paused-WIP manifests remain included. The coordinator additionally excluded the exact `paused-wip/source-wip.patch` from publication after two URL-userinfo pattern candidates; its cloud bytes remain unchanged and its path/size/SHA256/reason are retained in the selection ledger. Source commits remain reviewable through Git.

Prepare a provisional inventory or offline privacy assessment while files are still arriving (each output path must be new):

```sh
python3 /workspace/validation/asset-accuracy-publication-20261015/package_evidence.py plan --output /workspace/validation/asset-accuracy-publication-20261015/plan-01.json
python3 /workspace/validation/asset-accuracy-publication-20261015/package_evidence.py privacy --output /workspace/validation/asset-accuracy-publication-20261015/privacy-01.json
```

Privacy output contains only candidate categories, relative file locations/line numbers and complete file hashes; it never includes matched values or excerpts. Pattern matches can be synthetic fixture strings. PNGs are included as requested, but the scanner does not inspect pixels. Keep their existing local visual-review evidence and report that limitation. The coordinator requires **zero detector candidates in the final selected files**. A review attachment cannot bypass this rule; report locations and obtain the coordinator's selection decision without printing values or editing original logs.

The initial scan found two URL-userinfo shapes in `paused-wip/source-wip.patch`. The coordinator approved excluding that entire source patch from publication while retaining its cloud bytes and path/hash/size/reason in the ledger. `privacy-01.json` is the original value-free candidate report. The next strict scan rejected changing input files; `privacy-02-failed.json`/`.log` preserve that observed failure. There is no final zero-candidate assertion yet. All application/native failure logs remain selected; no test failure has been removed for privacy or size.

Only after the coordinator's final instruction, run (replace `FULL_FROZEN_SHA` with its final 40-character value; do not reuse an earlier source label):

```sh
python3 /workspace/validation/asset-accuracy-publication-20261015/package_evidence.py pack --validation-complete --frozen-head FULL_FROZEN_SHA --output /workspace/validation/asset-accuracy-publication-20261015/final-01
# Verify each archive named in final-01/publication-set.json, for example:
python3 /workspace/validation/asset-accuracy-publication-20261015/package_evidence.py verify --output /workspace/validation/asset-accuracy-publication-20261015/final-01/evidence.tar.gz
```

The packer reads source parents/files using `O_NOFOLLOW`, rejects unsafe relative paths and nonregular archive entries, never extracts links, and records SHA256 for every original evidence byte. It checks inventory identity/size/mtime/ctime before and after capture, rehashes bytes while writing, and fails if writers change the selected evidence. Output must be a new directory; temporary failed output is removed without touching source records.

The archive contains a readable `INDEX.md`, complete `manifest.json`, GNU-style `SHA256SUMS`, a selection/exclusion ledger, the exact packer/policy, privacy assessment and untouched raw evidence. `SHA256SUMS` covers every member except itself, including `manifest.json`; the archive's external hash covers everything. Run `sha256sum -c SHA256SUMS` inside the extracted archive root to recheck it. Every archive is capped at 20 MiB. A larger combined result is split into texts and screenshots archives; an oversized category is divided into numbered parts using a conservative 18 MiB raw-byte budget. Each part uses its own internal root, index, manifest and checksum set, and the publication-set ledger proves that every selected original file is covered exactly once. No historical runtime failure is dropped for size.

Packaging never treats compilation or a listed test as a runtime pass. Browser synthetic IPC is not a real Tauri/database/provider run, and temporary SQLite fixtures are not the user's database or Mac. The final application result must be reported from the original command/result records after they finish.
