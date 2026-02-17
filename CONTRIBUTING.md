# Contributing to r8r

We welcome contributions! Whether it's bug reports, feature requests, documentation improvements, or code contributions.

## Contributor License Agreement (CLA)

Before we can accept your first pull request, you must sign our [Contributor License Agreement](CLA.md).

**Why?** r8r is dual-licensed (AGPL-3.0 open source + commercial). The CLA ensures we can legally accept your contribution and continue offering both licenses. You retain full copyright ownership of your work.

**How to sign:**

1. Open your pull request
2. The CLA Assistant bot will comment with a signing link
3. Click the link and authenticate with GitHub
4. Done! (Takes under 1 minute)

You only need to sign once. All future contributions are covered.

## Getting Started

### Prerequisites

- Rust 1.75+ (install via [rustup](https://rustup.rs/))
- Git

### Build

```bash
git clone https://github.com/qhkm/r8r.git
cd r8r
cargo build
```

### Run Tests

```bash
cargo test
```

### Check Code Quality

```bash
cargo clippy
cargo fmt -- --check
```

## Submitting Changes

1. Fork the repository
2. Create a feature branch (`git checkout -b feat/my-feature`)
3. Make your changes
4. Ensure tests pass: `cargo test`
5. Ensure clean lints: `cargo clippy` and `cargo fmt`
6. Commit with a descriptive message
7. Open a pull request against `main`

## Commit Messages

Follow conventional commits:

- `feat:` new feature
- `fix:` bug fix
- `docs:` documentation changes
- `style:` formatting (no code change)
- `refactor:` code restructuring
- `test:` adding or updating tests
- `chore:` maintenance tasks

## Code Guidelines

- Follow existing code patterns and style
- Add tests for new functionality
- Keep PRs focused — one feature/fix per PR
- Update documentation if behavior changes

## Reporting Issues

- Use GitHub Issues
- Include: steps to reproduce, expected behavior, actual behavior
- Include: OS, Rust version, r8r version

## Questions?

Open a GitHub Discussion or Issue. We're happy to help!
