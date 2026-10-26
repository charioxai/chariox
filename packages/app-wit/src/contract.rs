use wit_parser::{PackageId, Resolve, SourceMap};

const DEPENDENCIES: &[(&str, &str)] = &[
    (
        "deps/cli-0.2.0/package.wit",
        include_str!("../wit/deps/cli-0.2.0/package.wit"),
    ),
    (
        "deps/clocks-0.2.0/package.wit",
        include_str!("../wit/deps/clocks-0.2.0/package.wit"),
    ),
    (
        "deps/filesystem-0.2.0/package.wit",
        include_str!("../wit/deps/filesystem-0.2.0/package.wit"),
    ),
    (
        "deps/http-0.2.0/package.wit",
        include_str!("../wit/deps/http-0.2.0/package.wit"),
    ),
    (
        "deps/io-0.2.0/package.wit",
        include_str!("../wit/deps/io-0.2.0/package.wit"),
    ),
    (
        "deps/random-0.2.0/package.wit",
        include_str!("../wit/deps/random-0.2.0/package.wit"),
    ),
    (
        "deps/sockets-0.2.0/package.wit",
        include_str!("../wit/deps/sockets-0.2.0/package.wit"),
    ),
    (
        "deps/cli-0.3.0/package.wit",
        include_str!("../wit/deps/cli-0.3.0/package.wit"),
    ),
    (
        "deps/clocks-0.3.0/package.wit",
        include_str!("../wit/deps/clocks-0.3.0/package.wit"),
    ),
    (
        "deps/filesystem-0.3.0/package.wit",
        include_str!("../wit/deps/filesystem-0.3.0/package.wit"),
    ),
    (
        "deps/http-0.3.0/package.wit",
        include_str!("../wit/deps/http-0.3.0/package.wit"),
    ),
    (
        "deps/random-0.3.0/package.wit",
        include_str!("../wit/deps/random-0.3.0/package.wit"),
    ),
    (
        "deps/sockets-0.3.0/package.wit",
        include_str!("../wit/deps/sockets-0.3.0/package.wit"),
    ),
];

pub(super) fn resolve() -> (Resolve, PackageId) {
    with_app(include_str!("../wit/app.wit"))
}

pub(super) fn with_app(app: &str) -> (Resolve, PackageId) {
    let mut resolve = Resolve::default();
    let mut main = SourceMap::default();
    main.push_str("app.wit", app);
    main.push_str("admission.wit", include_str!("../wit/admission.wit"));
    let main = main.parse().expect("embedded App WIT parses");
    let mut dependencies = Vec::new();
    for (path, contents) in DEPENDENCIES {
        let mut source = SourceMap::default();
        source.push_str(path, *contents);
        dependencies.push(source.parse().expect("vendored WASI WIT parses"));
    }
    let package = resolve
        .push_groups(main, dependencies)
        .expect("embedded WIT dependencies resolve");
    (resolve, package)
}
