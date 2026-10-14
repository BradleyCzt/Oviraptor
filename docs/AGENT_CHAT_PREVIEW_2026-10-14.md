# Agent-style Chat preview

This preview updates the independent Chat workspace inside the existing Oviraptor application. Assets, Scanning, project ownership and configuration remain part of the same host.

## Interface

- The left navigation starts collapsed. Expanding it exposes the current workspace and conversation history in one sidebar.
- The main chat area uses a centered reading column and a bounded transcript scroller. The composer stays below the transcript.
- Conversation management and linked plugin records are available through compact controls rather than permanent dashboard panels.
- Light and dark themes, accessible icon labels, keyboard focus and narrow-screen drawer behavior are included.
- Enter sends, Shift+Enter adds a line, and IME composition does not submit. The control distinguishes sending, local-only saving, explicit retry and stopping a reply.

## Message presentation

Assistant messages support headings, emphasis, lists, quotes, tables, links, inline code and fenced code blocks with highlighting. Code and complete message source can be copied. User messages remain literal text.

Raw HTML is not executed. Links are restricted to supported safe schemes; embedded images are represented by labels instead of automatically fetching remote resources. Large messages and code blocks have parsing/highlighting limits and fall back to readable text. An unfamiliar language still displays its source.

## Steer: next-reply direction

The current provider integration cannot inject instructions into an already running request. Steer therefore queues one bounded follow-up for the selected conversation while the current reply is running. The UI identifies it as a next-reply direction.

- Queue acceptance means the follow-up was stored locally, not that the provider has received it.
- Automatic dispatch is limited to a known successful completion of the original reply in the same mounted conversation selection.
- Failure, cancellation, uncertain outcomes, navigation or application reload pause automatic dispatch for explicit review.
- Retrying an uncertain dispatch reuses its stored operation identity and exact payload. It does not silently create another request.
- Local storage failures remain visible and do not count as successful queue acceptance.
- Scoped browser Web Locks serialize queue changes across windows. If that capability is unavailable or busy, the direction stays in the composer with a visible error; ordinary chat without a saved pending direction remains usable. A conversation with an unresolved saved direction requires safe recovery before further sends.

This is not live token streaming, tool execution or autonomous control of the Assets/Scanning plugins. Replies are displayed when the existing backend commits them. UI rendering does not imply that a model obeyed a direction.

## Preview validation and installation

The build artifact includes a manifest and checksums for the exact source commit. Its GitHub Actions run records the checks executed for that build, including synthetic browser scenarios where available. Synthetic IPC tests do not establish native IPC, provider integration or real scanning correctness.

The macOS preview targets Apple Silicon and is ad-hoc signed, without Apple notarization. Quit an older running copy and back up existing Oviraptor data before trying it; the app identifier and existing data locations are unchanged. External worker runtimes such as Node.js are not bundled.

The dependency audit still reports pre-existing advisories in the framework/build dependency tree. This change does not claim the whole repository is vulnerability-free; added Markdown/highlighting dependencies were checked separately.
