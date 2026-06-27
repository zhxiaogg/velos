# Velos — Engineering Guidelines

Velos is a Kubernetes-style control plane that manages the lifecycle of containers
(Apple Containerization micro-VMs) on a pool of registered remote macOS workers,
exposed over a RESTful API.

These principles are adopted from the `zhxiaogg/hackamore` project and are the
canonical design philosophy for this repository. They override convenience when in
conflict.

## Design Principles

1. **Semantic types over convenient types.** Types encode domain intent, not just
   data shape. `WorkerName`, `ContainerUid`, `ResourceVersion`, `BootstrapToken`,
   and `Secret` are not `String`/`u64`. If reusing an existing type would let a
   caller pass something semantically wrong, define a new type. The name of a type
   is part of its contract. A credential `Secret` is not a `String`; it never
   `Display`s, `Debug`s, or logs its contents.

2. **Make illegal states unrepresentable.** Use sum types (enums / tagged unions)
   to eliminate invalid combinations at the type level. `ContainerPhase`,
   `RestartPolicy`, and `WatchEvent` are sum types. Prefer exhaustive `match` over
   runtime guards — the compiler enforces completeness, not tests. No wildcard
   enum match arms.

3. **Deep modules.** Narrow public interface, deep implementation. The scheduler's
   public surface is essentially `fn schedule(unbound, workers) -> Option<WorkerName>`;
   the storage layer hides SQLite entirely behind a `Store` trait; the runtime hides
   the `container` CLI behind a `ContainerRuntime` trait. Every abstraction boundary
   should ask: what mistakes does this prevent, and what complexity does it hide?

4. **Compile-time over runtime enforcement.** Validate invariants at construction
   (admission), not scattered at call sites. Lints, type constraints, and the type
   system catch mistakes before production.

5. **Functional / immutable core, side effects at the edges.** Scheduling and
   reconciliation *decisions* are pure functions `(observed state) -> intended
   actions`, unit-testable in isolation. The actuators — datastore writes and
   `container` CLI calls — are the only side-effecting code. Mutation is local and
   obvious.

6. **Fail closed.** The default is to reject. Unknown/expired/revoked token → no
   registration. Ambiguous or expired lease → worker `NotReady`. Ambiguous runtime
   state → `Failed`, never assumed-running. Admission rejects malformed specs.
   There is no bypass and no anonymous path. Bounded actions (truncation, sampling,
   skipped items) are logged with counts — never silently dropped (no-silent-caps).

7. **Protocol types are not storage types.** Wire formats evolve at the speed of
   the interface contract; persisted structures at the speed of data migrations.
   Never conflate them. fluorite-generated types are the **wire contract only**.
   The datastore persists each object as an **opaque serialized document** plus
   hand-written **index columns** (`name`, `uid`, `resourceVersion`, `labels`,
   `spec.nodeName`). **Never use fluorite for persisted data structures.**

## Protocol Models & fluorite Conventions

- Use [fluorite](https://github.com/zhxiaogg/fluorite) to generate all protocol /
  wire types — anything transported between modules or between server and clients.
- Define schemas as `.fl` files under `crates/models/fluorite/` (inside the
  models crate, so the crate is self-contained).
- `velos-models` runs `fluorite_codegen` in `build.rs` and exposes generated types
  via `velos_models::<package>::*`.
- Generated types derive `Debug`, `Clone`, `PartialEq`, `Serialize`, `Deserialize`,
  `JsonSchema` (the `schemars` derive feeds the OpenAPI document).
- Add hand-written convenience constructors/methods in `crates/models/src/lib.rs`
  (not in the schema).
- fluorite is a pure data-type IDL: it does not describe REST endpoints. Resource
  *types* live in `.fl`; the REST routing is hand-written (axum).
- fluorite uses a single namespace across packages — type names must be unique
  project-wide (e.g. `WorkerSpec`, `ContainerSpec`, not two `Spec`s).

## Coding Conventions & Enforcement

Workspace clippy lints (mandatory, production code):

```toml
[workspace.lints.clippy]
unwrap_used             = "deny"
expect_used             = "deny"
panic                   = "deny"
wildcard_enum_match_arm = "deny"
```

Test code opts out per-file with `#![cfg_attr(test, allow(...))]`.

Error handling: `thiserror` for typed library errors at boundaries; `anyhow` in
binaries. No `unwrap`/`expect`/`panic` in production code (lint-enforced).

## Testing

- Unit tests live alongside source under `#[cfg(test)] mod tests` in the same `.rs`
  file.
- Full-stack end-to-end tests (spin up the apiserver + a fake `ContainerRuntime`) go
  in the `velos-tests` crate.

## Development Workflow

Pre-PR gate — `make check`:

```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```
