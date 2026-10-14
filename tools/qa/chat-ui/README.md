# Synthetic chat browser acceptance

This fixture compiles the actual Vue app and renders it in Chromium. All Tauri IPC is intercepted with synthetic project/history/reply data. Unimplemented native operations fail closed; non-local network requests are aborted. There are no credentials, provider calls, scan targets, user files, or Mac access. Writes affect only in-memory fixtures.

Do not commit generated bundles/screenshots or install test dependencies in the app package. In CI from repository root:

    npm ci --ignore-scripts
    npm ci --prefix tools/qa/chat-ui --ignore-scripts --no-audit --no-fund
    tools/qa/chat-ui/node_modules/.bin/playwright install --with-deps chromium
    export OVIRAPTOR_REPO_ROOT="$GITHUB_WORKSPACE"
    export OVIRAPTOR_QA_OUTPUT="$RUNNER_TEMP/chat-ui-qa"
    node tools/qa/chat-ui/build.mjs
    node tools/qa/chat-ui/browser-qa.cjs

Upload OVIRAPTOR_QA_OUTPUT as an Actions artifact even on failure. browser-results.json is the assertion report; screenshots/ contains actual browser captures, not serialized DOM. Inspect screenshots to assess visual quality; assertions only cover selected layout/interaction properties.

An installed Chromium can be used without a browser download by setting `OVIRAPTOR_QA_CHROMIUM=/usr/bin/chromium` (or its actual executable path). The report records the browser version and executable selection. `OVIRAPTOR_QA_SCENARIOS='^(10-|11-|12-|13-|14-)'` optionally selects scenarios for diagnosis; acceptance requires a full unfiltered run. Record the source commit with `GITHUB_SHA=$(git rev-parse HEAD)`. Keep output outside tracked source, for example `/workspace/validation/browser`.

The script tests collapsed shell, expansion/collapse with unsent draft preservation, single composer, rich content, safe links/raw HTML, keyboard/IME guard, uncertain retry idempotency, reply stop, save-only, long-message containment, wide/narrow viewports and scroll geometry. It does not prove native Tauri, SQLite persistence or actual model behavior.

The 1440×1000 captures represent a desktop viewport above the application’s 1180px native minimum width. The 390×844 captures are web stress tests below the native-supported window size, explicitly testing CSS containment rather than claiming native phone support. Both Cloud and Codex themes are captured.

Steer coverage treats Steer as a locally persisted, queued next-turn message. It verifies no immediate send while running, exact queued text/status, composer + independent Stop in a short narrow viewport, safe-completion dispatch, and Stop pausing before an explicit later send. This does not claim live mid-generation provider injection. Clipboard checks use an isolated synthetic clipboard sink; they prove browser handler behavior and exact copied source, not the host OS clipboard.

The browser fixture uses Chromium's native Web Locks on localhost. A two-page same-origin scenario checks competing queue acceptance and preservation of the losing draft; it does not replace the browser lock API. All source inputs are SHA-256 recorded before/after the build and after browser testing.

Navigation acceptance also seeds saved Assets and Scanning routes and verifies that startup still opens only Chat with no plugin initialization IPC, including HackerOne activity reads when configured. It checks actual expanded-sidebar geometry and DOM order (Chat, conversation history, bottom plugin entries, workspace), then explicitly opens each plugin and returns to the preserved conversation list. These fixtures perform read-only synthetic plugin initialization, never a scan or provider call.

Business-module scenarios switch projects through the real workspace selector and verify scoped history, drafts, grants, and saved tool activity. Access scenarios exercise explicit disclosure, directory staging/cancel/confirmation, revocation during an active synthetic reply, and ordinary chat with Agent disabled. Changing the synthetic provider identity or selected skill version is followed by real conversation navigation and backend-response reads; the actual access panel must require renewed confirmation and allow a disabled selected skill to be removed. This validates frontend behavior against an explicit synthetic contract; it does not prove native recipient hashing, filesystem safety, or mid-flight revocation enforcement.

The Skills scenario opens the independent navigation entry, previews a synthetic directory, confirms import, preserves the preview after a simulated import failure, retries explicitly, reads instructions and inert script resources, disables the imported skill, and returns to saved Chat. Import and enable operations affect only the fixture's in-memory package list. No real package is installed or script run. The final report includes totals for runtime errors, blocked network requests, unknown IPC, and attempted external fetches.
