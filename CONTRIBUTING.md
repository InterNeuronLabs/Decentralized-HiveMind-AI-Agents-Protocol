# Contributing

Thank you for your interest in contributing! This project is Apache 2.0
licensed. By submitting a PR you agree to the [CLA](CLA.md).

## Before you start

- Open an issue to discuss significant changes before filing a PR.
- For security vulnerabilities, **do not open a public issue**. Email the
  maintainer directly (see the GitHub profile).

## Development setup

```bash
# Prerequisites: Rust stable, PostgreSQL 15+, Redis 7+
cargo build --workspace
cargo test  --workspace

# For the orchestrator you need a running Postgres + Redis:
cp .env.example .env  # fill in your values
cargo run -p orchestrator
```

## Code style

- `cargo fmt` is enforced in CI.
- `cargo clippy -- -D warnings` is enforced in CI.
- `#![forbid(unsafe_code)]` on all crates except direct FFI boundaries.
- No `unwrap()` in non-test production paths — use `?` or `anyhow::bail!`.

## Commit messages

Follow [Conventional Commits](https://www.conventionalcommits.org/):
`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`, `security:`, etc.

## Pull requests

- Keep PRs focused and small.
- Include tests for new behaviour.
- Update `.env.example` if you add new config variables.
- Admin / private infrastructure changes go in `private/` (git-ignored).
