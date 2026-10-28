# Rust App SDK

The bindings come directly from `packages/app-wit/wit`, the async R1 contract.
Use the checked-in `template` as a source-based Todo backend. `Guest` is the
exported handlers trait; `bindings::export!(App with_types_in
chariox_app_sdk::bindings)` emits the component exports. `cancellable(cancel,
work)` drops pending work and owned streams when the cancellation future wins.
`get_json` retains the host state version for compare-and-swap transactions.

Build and prepare the output using [the shared compiler instructions](../app-sdk-tools/README.md).
This SDK currently builds from the monorepo alongside its canonical WIT; it is
not a standalone crates.io distribution. Keep the SDK and WIT from the same
revision. The two Todo operations use durable state and optimistic checks; a
concurrent write may return the host's conflict error instead of losing data.
