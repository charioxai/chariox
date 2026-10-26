//! R6 adds broker-backed WASI bindings to this linker. This worker itself
//! links only Chariox imports and grants no ambient host authority.
wasmtime::component::bindgen!({
    path: "../app-wit/wit",
    inline: r#"
        package chariox:worker;
        world worker {
            import chariox:app/registry@1.0.0;
            import chariox:app/state@1.0.0;
            import chariox:app/schedule@1.0.0;
            import chariox:app/events@1.0.0;
            import chariox:app/connections@1.0.0;
            import chariox:app/files@1.0.0;
            import chariox:app/http@1.0.0;
            import chariox:app/log@1.0.0;
            import chariox:app/host@1.0.0;
            import chariox:app/validation@1.0.0;
            export chariox:app/handlers@1.0.0;
        }
    "#,
    world: "worker",
    imports: { "chariox:app/registry": trappable, "chariox:app/log": trappable },
    exports: { "chariox:app/handlers.register": async },
});
