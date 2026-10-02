# lemonfiber-request-gate

The request gate: the one path from the request service (Seerr) to Sonarr, Radarr and Jellyfin. It holds their credentials, publishes no port, answers a fixed list of calls it builds itself, and refuses everything else without forwarding it.

It is the source of `ghcr.io/lemonfiber/request-gate`, which [`lemonfiber-media-stack`](https://github.com/lemonfiber/lemonfiber-media-stack) runs as the service `request-gate`. Its decisions are [ADR-0032](https://github.com/lemonfiber/spec/blob/main/00-overview/decisions/0032-the-request-service-reaches-the-arrs-through-a-gate.md) and [ADR-0033](https://github.com/lemonfiber/spec/blob/main/00-overview/decisions/0033-each-image-lemonfiber-builds-for-the-stack-has-its-own-repository.md); what it holds is in the specification's [`30-repos/lemonfiber-request-gate.md`](https://github.com/lemonfiber/spec/blob/main/30-repos/lemonfiber-request-gate.md).

It is released on lemonfiber's version train, tagged before the core, and the stack pins the digest each tag publishes.

## Building

```sh
cargo test
docker build -t request-gate .
```

The image runs as a non-root user on a distroless base, with a read-only root and no
kernel capabilities, and publishes no port. Its health check is `request-gate health`,
the binary asking itself over loopback.

## Licence

[Hippocratic License 3.0](LICENSE), as every repository of lemonfiber's.
