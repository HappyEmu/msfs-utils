# Repository Guidelines

## Project Structure & Module Organization

This repository is a Rust 2024 workspace with four crates:

- `msfs-async/`: runtime-independent asynchronous SimConnect client; Windows-specific implementation lives in `src/windows.rs`.
- `msfs-sync/`: blocking facade over `msfs-async`.
- `msfs-async-derive/`: procedural macros plus unit and integration tests.
- `msfs-replay/`: platform-independent CSV loading and flight-pose interpolation.

Each crate keeps library code in `src/`, documentation in `README.md`, and runnable samples in `examples/` where applicable. `checkouts/msfs-rs/` is an excluded upstream dependency checkout; avoid changing it unless the task explicitly concerns that dependency. `target/` contains generated build artifacts and must not be committed.

## Build, Test, and Development Commands

- `cargo check --workspace`: type-check every workspace crate quickly.
- `cargo build --workspace`: compile all libraries in debug mode.
- `cargo test --workspace`: run unit and integration tests, including macro-generation and replay interpolation coverage.
- `cargo fmt --all -- --check`: verify standard Rust formatting.
- `cargo clippy --workspace --all-targets -- -D warnings`: run lint checks and reject warnings.
- `cargo run -p msfs-async --example request_once`: run an async example on Windows with MSFS and its SDK installed.

SimConnect clients are Windows-only; replay and macro tests should remain testable on other platforms.

## Coding Style & Naming Conventions

Follow `rustfmt` defaults (four-space indentation and trailing commas). Use `snake_case` for modules, functions, fields, and test names; `UpperCamelCase` for types and traits; and `SCREAMING_SNAKE_CASE` for constants. Document public APIs with `///`, keep `unsafe` invariants explicit, and isolate platform code behind `#[cfg(windows)]`.

## Testing Guidelines

Place focused unit tests in a local `#[cfg(test)] mod tests`; use `crate/tests/*.rs` for public macro or API behavior. Name tests after the behavior being guaranteed, such as `generated_types_implement_the_safe_api_contracts`. Add regression tests for parsing boundaries, interpolation, wire-layout validation, and platform-neutral logic.
