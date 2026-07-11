# Network

Egress control. Planned, not enforced yet — profiles already record
`allowedDomains`, but nothing acts on them today, and the sandbox leaves the
network open.

The plan, kept out of v0 for simplicity:

- **OS layer: only the proxy is reachable.** One static rule, identical for
  every package. macOS: Seatbelt denies network except localhost:port.
  Linux: seccomp user-notify on `connect()` allows only the proxy address,
  plus classic seccomp blocking UDP/raw sockets and `io_uring` (which could
  otherwise send without `connect()`).
- **Proxy: the policy point.** A small CONNECT proxy; hostname allowlist
  from the profile in enforce mode, allow-and-log in observe mode. No MITM —
  TLS passes through opaque. The proxy resolves DNS, so the child needs
  none.

Validated already on macOS: Seatbelt blocks direct egress while allowing a
localhost proxy port, without sudo; SBPL accepts `localhost:PORT` but not
numeric IPs; fspy's preload survives inside Seatbelt if the profile allows
`ipc-posix-shm*`.
