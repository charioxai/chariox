//! Small source scaffold; signing keys are never generated or copied here.

use std::{collections::BTreeMap, path::Path};

use serde::Serialize;
use serde_json::json;

use super::{fs, v2::generate_manifest, ManifestOptions, PublisherFile};
use crate::{json as package_json, v2::ManifestV2, Limits, Result};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldReport {
    pub status: &'static str,
    pub directory: String,
    pub manifest: ManifestV2,
}

/// All validation happens before creating the destination. Creation refuses an
/// existing directory, including an empty one or a symlink. A filesystem error
/// can leave an incomplete new scaffold; it never deletes or overwrites files.
pub fn create_scaffold(
    directory: &Path,
    app_id: &str,
    version: &str,
    publisher: &PublisherFile,
    kernel_protocol: u32,
    limits: &Limits,
) -> Result<ScaffoldReport> {
    publisher.trusted_publisher()?;
    let mut options = ManifestOptions::new(
        app_id.into(),
        version.into(),
        publisher.publisher.clone(),
        kernel_protocol,
    );
    options.runtime_entry = "runtime/app.wasm".into();
    options.tools = Some("schemas/tools.json".into());
    let manifest = generate_manifest(options, limits)?;
    let tools = json!({"tools":[{
        "name":"greet", "description":"Return a greeting for a name.",
        "inputSchema":{"type":"object","properties":{"name":{"type":"string","maxLength":200}},
            "required":["name"],"additionalProperties":false},
        "outputSchema":{"type":"object","properties":{"greeting":{"type":"string"}},
            "required":["greeting"],"additionalProperties":false}
    }]});
    let files = BTreeMap::from([
        ("app.json", package_json::canonical(&manifest)?),
        ("publisher.json", package_json::canonical(publisher)?),
        ("src/app.ts", b"import { defineApp } from '@chariox/app-sdk-wasm';\nexport const handlers = defineApp({ tools: { async greet({ name }: { name: string }) { return { greeting: `Hello, ${name}!` }; } } });\n".to_vec()),
        ("bundle/schemas/tools.json", package_json::canonical(&tools)?),
        ("bundle/ui/index.html", b"<!doctype html>\n<html lang=\"en\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>Hello Chariox</title>\n<main><h1>Hello Chariox</h1><p>Your App is ready to customize.</p><p>The App runtime exposes a <code>greet</code> tool for Chariox agents.</p></main></html>\n".to_vec()),
        (".gitignore", b"*.cxapp\n*.key\n*.pem\n.env\n.env.*\nnode_modules/\n".to_vec()),
        ("README.md", README.as_bytes().to_vec()),
    ]);
    fs::create_source_tree(directory, &files)?;
    Ok(ScaffoldReport {
        status: "created-locally",
        directory: directory.to_string_lossy().into_owned(),
        manifest,
    })
}

const README: &str = r#"# Your Chariox WASM App

`src/app.ts` implements the `greet` tool declared in `bundle/schemas/tools.json`.
Compile it with `packages/app-sdk-tools/build-js.mjs` into
`bundle/runtime/app.wasm` before packing. The SDK build tool resolves the WASM
SDK and checks the component's imports; Node is a build tool only.
`bundle/ui/index.html` is the view. `app.json` is a contract-2 manifest;
`publisher.json` holds only the public identity.

Keep signing keys outside the source tree. After compiling:

```sh
chariox app pack --bundle bundle --manifest app.json --key /path/to/private-key --output example.cxapp
chariox app validate example.cxapp --trust publisher.json
chariox app inspect example.cxapp
```

Packing does not run a build, install dependencies, enroll a publisher or execute
an App. Complete the component build before packing; no engine is bundled.
"#;
