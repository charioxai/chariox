//! Small source scaffold; signing keys are never generated or copied here.

use std::{collections::BTreeMap, path::Path};

use serde::Serialize;
use serde_json::json;

use super::{fs, generate_manifest, ManifestOptions, PublisherFile};
use crate::{json as package_json, Limits, Manifest, Result};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldReport {
    pub status: &'static str,
    pub directory: String,
    pub manifest: Manifest,
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
    options.runtime_entry = "runtime/main.mjs".into();
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
        ("bundle/runtime/main.mjs", b"export default function register(chariox) {\n  chariox.tools.register('greet', async ({ name }) => ({ greeting: `Hello, ${name}!` }));\n}\n".to_vec()),
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

const README: &str = r#"# Your Chariox App

`bundle/runtime/main.mjs` registers the `greet` tool declared in
`bundle/schemas/tools.json`. `bundle/ui/index.html` is the App view.
`app.json` is generated from the selected SDK/App contract and CLI protocol.
`publisher.json` contains only your public publisher identity.

Keep your private signing key in its protected directory outside this project.
From this project directory, package the completed source bundle with:

```sh
chariox app pack --bundle bundle --manifest app.json --key /path/to/private-key --output example.cxapp
chariox app validate example.cxapp --trust publisher.json
chariox app inspect example.cxapp
```

Replace the key placeholder with your existing private key. No private key or
machine-specific path is stored in this scaffold. Packaging performs no npm
install, build script, network request or App execution. If your App needs a
build step, write its completed output into `bundle/` before packing.

Local validation checks the selected publisher and package bytes. It does not
enroll that publisher in a kernel, approve capabilities, or install the App.
"#;
