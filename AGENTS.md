# AGENTS.md — lemonfiber-request-gate

Guidance for any AI agent (Cursor, Codex, Aider, Claude Code, …) working in this
repo.

> **Start at the report** of where every unreleased version stands: the summary
> of the newest run of the spec's [`state` workflow](https://github.com/lemonfiber/spec/actions/workflows/state.yml),
> or `just goals <version>` in a spec checkout.
> **Then the rules every repository shares:**
> [working in the repositories](https://github.com/lemonfiber/spec/blob/main/50-governance/working-in-the-repositories.md)
> and [the rules for agents](https://github.com/lemonfiber/spec/blob/main/50-governance/ai-contributors.md).
> This file holds only what is true of `lemonfiber-request-gate`.

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
