# Security Policy

## Supported versions

Security fixes go into the latest release. r8r is pre-1.0, so we do not backport fixes to older minor versions. Upgrade to the newest `0.8.x` to receive them.

| Version | Supported |
|---------|-----------|
| 0.8.x   | Yes       |
| < 0.8   | No        |

## Reporting a vulnerability

**Please do not open a public issue for security problems.**

Report privately through either channel:

- **GitHub:** [open a private security advisory](https://github.com/qhkm/r8r/security/advisories/new) (preferred)
- **Email:** hello@r8r.sh, with "SECURITY" in the subject

Please include:

- the affected version (`r8r --version`) and how r8r is deployed (binary, Docker, `cargo install`)
- steps to reproduce, or a proof of concept
- the impact you expect (for example, data exposure, code execution, or SSRF)

## What to expect

- Acknowledgement within 3 business days.
- An initial assessment within 7 days, including whether we accept the report and how severe we judge it.
- A fix in a new release as soon as practical, usually within 30 days for high-severity issues. We will agree a disclosure date with you and credit you in the release notes and advisory, unless you prefer to stay anonymous.

## Scope

In scope: the `r8r` and `r8r-mcp` binaries, the crate published on crates.io, the `ghcr.io/qhkm/r8r` image, and the install script at `https://r8r.sh/install.sh`.

Areas we especially want to hear about:

- sandbox escapes from the `sandbox` node (Docker, Firecracker, WASM backends)
- bypasses of SSRF protection in HTTP nodes and redirects
- authentication or authorization bypasses in the REST API, MCP server, or bridge WebSocket
- credential exposure, including secrets leaking into logs, execution history, or REPL transcripts
- path traversal in the dashboard or file-handling nodes

Out of scope:

- risks that require an already-compromised host or a malicious operator
- deliberate configuration choices, such as `R8R_ALLOW_INTERNAL_URLS=true` allowing internal URLs
- vulnerabilities in third-party services that r8r integrates with
- reports from automated scanners without a working proof of impact
