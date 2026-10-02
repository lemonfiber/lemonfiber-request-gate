# AGENTS.md — lemonfiber-request-gate

Guidance for any AI agent (Cursor, Codex, Aider, Claude Code, …) working in this
repo.

> **Common rules for every lemonfiber repo are canonical in the spec:**
> [50-governance/ai-contributors.md](https://github.com/lemonfiber/spec/blob/main/50-governance/ai-contributors.md).
> Read them. This file is the `lemonfiber-request-gate`-specific header only.

## What this repo is

The request gate: the request service's one path to Sonarr, Radarr and Jellyfin (ADR-0032). One Rust binary crate and one distroless image that publishes no port. See the spec:
[`30-repos/lemonfiber-request-gate.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-request-gate.md).

## The one rule you cannot break here

**Nothing is forwarded as it came.** Every call on the list is built by the gate itself from the parameters and body fields ADR-0032 names, and every other call is refused with 403 and recorded. A route that passes a request, a header or a token through is a defect however it is reached.

## Code standards (enforced)

- `unsafe` is **forbidden**. No `unwrap`/`expect`/`panic`/`todo` in non-test code.
- **No lint suppressions in `src/`**: change the code or the rule, never `#[allow]`.
- Every commit is signed and cites the requirement it serves (`Spec: <ID>`).
- The image stays distroless and non-root, with nothing in it but the binary.
