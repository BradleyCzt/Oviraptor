# Bundled Compound Engineering Skills

Upstream: https://github.com/EveryInc/compound-engineering-plugin
Pinned commit: 50532b00de523980010d97fe950965b35382a3b7
License: MIT; exact upstream license is also inside the payload.

`compound-engineering.json.gz` preserves the 36 Skill directories (442 files) and root LICENSE (443 files total). The deterministic gzip contains a versioned JSON manifest with per-file SHA-256 and exact Base64 content. It is inert data, not a plugin installer or executable entrypoint. Bundled scripts are retained for inspection but are not executed by import or Skill reads.

Compressed SHA-256: 122f5e82ee2e64e7c32034b62727d343cc20f6d1f02f3f7c14ac73f5c696668a
Decoded manifest SHA-256: 93bf449c79c6ad94fde5ed485f3c72e11b73316b8371f6957b980cbfb1f6e692
Decoded manifest bytes: 7,372,911

Source verification: every one of the 1,488 regular files in the downloaded upstream archive matched its Git blob hash from the pinned recursive Git tree. Only the Skill directories and LICENSE are included in this distribution. Source integrity is not a claim that every workflow has executable host support.
