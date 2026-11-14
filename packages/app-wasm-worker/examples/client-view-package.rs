//! Sign an App directory with a self-contained client view as its UI entry, for the native-open drill.
//! usage: client-view-package <core.wasm> <view html> <app directory> <output directory>
//! The manifest, the tool and the action declarations are the App's own (`app.json`, `schemas/`);
//! only the publisher (the drill's, which the drill enrolls like any other) is not. The package is written as `<last part of the app id>.cxapp`.
use chariox_app_package::{
    v2::{self, ManifestV2},
    Limits,
};
use std::{collections::BTreeMap, path::PathBuf};

mod view_signing;

fn main() {
    let mut args = std::env::args_os().skip(1).map(PathBuf::from);
    let (core, view, app, output) = (
        args.next().expect("core module or component"),
        args.next().expect("view html"),
        args.next().expect("app directory"),
        args.next().expect("output directory"),
    );
    let key = view_signing::dev_key();
    let mut manifest: ManifestV2 =
        serde_json::from_slice(&std::fs::read(app.join("app.json")).unwrap()).unwrap();
    manifest.publisher = chariox_app_package::Publisher {
        id: "com.example".into(),
        key_id: key.id.clone(),
        name: "Native view fixture".into(),
    };
    let mut files = BTreeMap::from([
        (
            manifest.runtime.component.clone(),
            view_signing::component(&core),
        ),
        (manifest.ui.entry.clone(), std::fs::read(view).unwrap()),
    ]);
    for declared in [&manifest.tools, &manifest.ui_actions]
        .into_iter()
        .flatten()
    {
        files.insert(declared.clone(), std::fs::read(app.join(declared)).unwrap());
    }
    std::fs::create_dir_all(&output).unwrap();
    let name = manifest.app_id.rsplit('.').next().unwrap().to_owned();
    std::fs::write(
        output.join(format!("{name}.cxapp")),
        v2::pack(&manifest, &files, &key.key, &[], &Limits::default()).unwrap(),
    )
    .unwrap();
    view_signing::write_publisher(
        &output,
        &manifest.publisher.id,
        &manifest.publisher.name,
        &key,
    );
}
