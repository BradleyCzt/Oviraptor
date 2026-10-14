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

The script tests collapsed shell, expansion/collapse with unsent draft preservation, single composer, rich content, safe links/raw HTML, keyboard/IME guard, uncertain retry idempotency, reply stop, save-only, long-message containment, wide/narrow viewports and scroll geometry. It does not prove native Tauri, SQLite persistence or actual model behavior.

The 1440×1000 captures represent a desktop viewport above the application’s 1180px native minimum width. The 390×844 captures are web stress tests below the native-supported window size, explicitly testing CSS containment rather than claiming native phone support. Both Cloud and Codex themes are captured.

Steer coverage treats Steer as a locally persisted, queued next-turn message. It verifies no immediate send while running, exact queued text/status, composer + independent Stop in a short narrow viewport, safe-completion dispatch, and Stop pausing before an explicit later send. This does not claim live mid-generation provider injection. Clipboard checks use an isolated synthetic clipboard sink; they prove browser handler behavior and exact copied source, not the host OS clipboard.

The browser fixture uses Chromium's native Web Locks on localhost. A two-page same-origin scenario checks competing queue acceptance and preservation of the losing draft; it does not replace the browser lock API. All source inputs are SHA-256 recorded before/after the build and after browser testing.
