# lemonfiber-request-gate

The request gate is a small service inside a [lemonfiber](https://github.com/lemonfiber/lemonfiber)
media stack. It sits between Seerr, the page where your household asks for films
and shows, and the services that act on those requests: Sonarr, Radarr and
Jellyfin.

This repository builds the container image `ghcr.io/lemonfiber/request-gate`.
You do not run it yourself: [`lemonfiber-media-stack`](https://github.com/lemonfiber/lemonfiber-media-stack)
runs it as the service `request-gate`, and `lemonfiber` writes its configuration.

## Why it exists

Seerr is a household-facing service: everyone at home can reach it. To pass a
request on, Seerr needs to call Sonarr, Radarr and Jellyfin. Given their API
keys directly, a fault in Seerr would hand anyone who reached it full control of
all three.

The gate holds those keys instead, and Seerr holds only a token for the gate.
The gate:

- answers a fixed list of calls, such as looking up a film, adding a series,
  removing one, or signing a household member in to Jellyfin;
- builds each upstream request itself from the parts of the call it reads,
  rather than forwarding what Seerr sent;
- refuses every other call and forwards none of it. It records each refusal,
  and each removal it passed on, without tokens, query strings or bodies;
- publishes no port. Only Seerr can reach it, over a network the two share.

The list of calls is in [`src/arr.rs`](src/arr.rs) (Sonarr and Radarr) and
[`src/jellyfin.rs`](src/jellyfin.rs).

## How the stack runs it

| | |
| --- | --- |
| Listens on | Port 5057 inside its container, on the stack's internal networks only |
| Configuration | `/config`, which `lemonfiber` fills: `upstreams.json` (where each upstream is, and its key), `tokens.json` (hashes of the tokens Seerr may present) and `record.json` (the refusals and removals, written by the gate) |
| Settings | `LEMONFIBER_REQUEST_GATE_CONFIG` moves the configuration directory; nothing else is configurable |
| Health check | `request-gate health`, which asks the running gate over loopback |
| Container | Distroless base, non-root user, read-only root filesystem, no kernel capabilities |

The compose entry and its networks are in
[`lemonfiber-media-stack`](https://github.com/lemonfiber/lemonfiber-media-stack).
The design is written up in the specification:
[`30-repos/lemonfiber-request-gate.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-request-gate.md).

## Building and testing

You need Rust (the version in [`rust-toolchain.toml`](rust-toolchain.toml)) and
Docker. The build fetches one crate from the
[`lemonfiber`](https://github.com/lemonfiber/lemonfiber) repository, at the
commit pinned in [`Cargo.toml`](Cargo.toml), so it needs network access the first time.

```sh
cargo test
cargo clippy --all-targets --locked -- -D warnings
docker build -t request-gate .
```

Images are published from version tags. Each tag builds `linux/amd64` and
`linux/arm64`, and `lemonfiber-media-stack` pins the digest it published.

## Contributing and security

Read the [contributing guide](https://github.com/lemonfiber/spec/blob/main/50-governance/contributing.md)
before opening a pull request. Report a vulnerability as the
[security policy](https://github.com/lemonfiber/.github/blob/main/SECURITY.md)
describes, not in a public issue.

## Licence

[Hippocratic License 3.0](LICENSE).
