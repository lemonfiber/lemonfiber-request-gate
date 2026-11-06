# lemonfiber-request-gate

The request gate is a small service inside a [lemonfiber](https://github.com/lemonfiber/lemonfiber)
media stack. It sits between Seerr, the page where your household asks for films
and shows, and the services that act on those requests: Sonarr, Radarr and the
media server, Jellyfin or Plex.

This repository builds the container image `ghcr.io/lemonfiber/request-gate`.
You do not run it yourself: lemonfiber installs it as the plugin `request-gate`,
from the `plugin.toml` here, and writes its configuration.

## Why it exists

Seerr is a household-facing service: everyone at home can reach it. To pass a
request on, Seerr needs to call Sonarr, Radarr and the media server. Given their
API keys directly, a fault in Seerr would hand anyone who reached it full control
of all three.

The gate holds those keys instead, and Seerr holds only a token for the gate.
The gate:

- answers a fixed list of calls, such as looking up a film, adding a series,
  removing one, or signing a household member in to Jellyfin;
- builds each upstream request itself from the parts of the call it reads,
  rather than forwarding what Seerr sent;
- refuses every other call and forwards none of it. It records each refusal,
  and each removal it passed on, without tokens, query strings or bodies;
- reaches the media server only over TLS at its internal TLS port, and sends a
  call there only to a certificate whose SHA-256 lemonfiber pinned with the route;
- reads Plex as Seerr's own Plex account, whatever token Seerr presented;
- publishes no port. Only Seerr can reach it, over a network the two share.

Where Plex is the media server, the gate also meets plex.tv, and only at its
`/users/account.json`. When Seerr's owner is your own Plex account, the gate asks
plex.tv whose a token Seerr presents is, once per token. When Seerr is set to reach
plex.tv through the gate, the gate answers plex.tv's calls in its place from what
lemonfiber hands it, giving every account an address under `plex.invalid` and never
one your household holds, and passes a member's own Plex sign-in on to plex.tv.

The list of calls is in [`src/arr.rs`](src/arr.rs) (Sonarr and Radarr),
[`src/jellyfin.rs`](src/jellyfin.rs), [`src/plex.rs`](src/plex.rs) and
[`src/plextv.rs`](src/plextv.rs) (plex.tv).

## How the stack runs it

| | |
| --- | --- |
| Listens on | Port 5057 inside its container, on the stack's internal networks only, for the request service; port 32400 for its Plex route, in Plex's own API; port 443 for plex.tv's calls over TLS, which the request service reaches by plex.tv's own names when it is set to; and port 5058 for lemonfiber's `request.gate` contract, published on loopback alone |
| Configuration | `/config`: the key lemonfiber asks the contract with (`lemonfiber.key`); the routes and tokens lemonfiber hands the gate, `upstreams.json` (where each upstream is, its key, for the media server the SHA-256 of each certificate its internal TLS port may present, and for Plex how Seerr is linked to it) and `tokens.json` (hashes of the tokens Seerr may present); `plex-tv/certificate.pem` and `plex-tv/key.pem`, the certificate lemonfiber makes for plex.tv's names, where Seerr reaches plex.tv through the gate; and `record.json` (the refusals and removals, written by the gate and answered over `request.gate`) |
| Settings | `LEMONFIBER_REQUEST_GATE_CONFIG` moves the configuration directory; nothing else is configurable |
| Health check | `request-gate health`, which asks the running gate over loopback |
| Version | `request-gate version`, which prints `request-gate` and its version to standard output |
| Container | Distroless base, non-root user, read-only root filesystem, no kernel capabilities |

lemonfiber writes the compose entry and its networks from `plugin.toml`.
The design is written up in the specification:
[`30-repos/lemonfiber-request-gate.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-request-gate.md).

## Building and testing

You need Rust (the version in [`rust-toolchain.toml`](rust-toolchain.toml)) and
Docker. The build fetches the contract, the adapter kit, the ports and the file
formats from the [`lemonfiber`](https://github.com/lemonfiber/lemonfiber)
repository, at the commit pinned in [`Cargo.toml`](Cargo.toml), so it needs network access the first time.

```sh
cargo test
docker build -t request-gate .
```

Images are published from version tags. Each tag builds `linux/amd64` and
`linux/arm64`, and `plugin.toml` pins the digest it published.

## Contributing and security

Read the [contributing guide](https://github.com/lemonfiber/spec/blob/main/50-governance/contributing.md)
before opening a pull request. Report a vulnerability as the
[security policy](https://github.com/lemonfiber/.github/blob/main/SECURITY.md)
describes, not in a public issue.

## Licence

[Hippocratic License 3.0](LICENSE).
