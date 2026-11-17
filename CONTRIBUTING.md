# Contributing to MySQL-Rusty

Thanks for your interest in contributing to **MySQL-Rusty**!

## Prerequisites

- **Rust toolchain:** `rustup` with stable Rust `1.85` or newer (`edition = "2024"`).
- **Linux system libraries (Ubuntu / Debian):** see the [System Prerequisites](./README.md#system-prerequisites-ubuntu--debian) section in `README.md` (plus `libdbus-1-dev` for `keyring`).

## Development Workflow

1. Read [`ARCHITECTURE_AND_PLAN.md`](./ARCHITECTURE_AND_PLAN.md), especially the **Golden Rules**:
   - Keep all code, comments, error messages, and documentation in **English**; user-facing UI strings use `rust_i18n::t!(...)` backed by `locales/en.yml` and `locales/es.yml`.
   - **Never block the UI thread**: run all network, database, and CLI I/O on `self.rt` (`Arc<tokio::runtime::Runtime>`) and send results back through `self.tx` as an `AsyncEvent` handled in `process_async_events`.
2. Create a feature or fix branch from `main`.
3. Before opening a pull request, verify that formatting, linting, and unit tests all pass:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test
```

(Run `cargo fmt --all` to auto-format your changes.)

## Commit Messages

This repository follows [Conventional Commits](https://www.conventionalcommits.org/) (e.g., `feat(editor): ...`, `fix(tls): ...`, `docs: ...`, `ci: ...`, `test: ...`, `chore: ...`).

## License

By contributing, you agree that your contributions will be dual-licensed under `MIT OR Apache-2.0` (see [`LICENSE-MIT`](./LICENSE-MIT) and [`LICENSE-APACHE`](./LICENSE-APACHE)).
