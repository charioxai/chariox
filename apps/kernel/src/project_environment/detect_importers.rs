//! MP-08 / MP-10 / MP-11: Data importers, never interpreters, installers or capability admission.
use super::detect_index::{credential_configuration_name, safe_metadata, skip, EvidenceFile};
use super::*;
use serde_json::Value;

pub(super) struct Importer {
    pub environment_id: String,
    pub proposals: std::collections::BTreeMap<String, EnvironmentProposal>,
    pub skips: Vec<EnvironmentItemResult>,
    pub code_folders: std::collections::BTreeSet<String>,
}
impl Importer {
    fn add(
        &mut self,
        file: &EvidenceFile,
        key: &str,
        title: &str,
        line: Option<u32>,
        spec: RequirementSpec,
    ) {
        if !safe_metadata(title) {
            self.skips
                .push(skip(&file.folder_id, &file.path, "protected_metadata"));
            return;
        }
        // Secret names are folder requirements with all actual use sites.
        let identity = if key.starts_with("secret:") {
            format!("{}\0{key}", file.folder_id)
        } else {
            format!("{}\0{}\0{key}", file.folder_id, file.path)
        };
        let id = project_environment_item_id(&self.environment_id, &identity);
        let scope = RequirementScope::Folder {
            folder_id: file.folder_id.clone(),
        };
        let origin = RequirementOrigin::Detected {
            folder_id: file.folder_id.clone(),
            relative_path: file.path.clone(),
            line,
            evidence_digest: String::new(),
        };
        if let Some(proposal) = self.proposals.get_mut(&id) {
            if !proposal.requirement.origins.contains(&origin) {
                proposal.requirement.origins.push(origin)
            }
            return;
        }
        // Existing IDs retain new origins even when new-proposal admission is full.
        if self.proposals.len() >= 2048 {
            self.skips
                .push(skip(&file.folder_id, &file.path, "proposal_limit"));
            return;
        }
        self.proposals.insert(
            id.clone(),
            EnvironmentProposal {
                proposal_id: id.clone(),
                requirement: Requirement {
                    requirement_id: id,
                    title: title.into(),
                    scope,
                    origins: vec![origin],
                    spec,
                    depends_on: vec![],
                    platform_variants: vec![],
                    required: false,
                    legacy_entry: None,
                },
            },
        );
    }
    fn software(
        &mut self,
        file: &EvidenceFile,
        name: &str,
        version: Option<&str>,
        line: Option<u32>,
        gui: bool,
    ) {
        if !identity(name) {
            return;
        }
        let version = version.filter(|v| version_token(v)).map(str::to_owned);
        self.add(
            file,
            &format!("software:{name}"),
            name,
            line,
            RequirementSpec::Software {
                identity: name.into(),
                version_constraint: version,
                platform: if gui && name == "ChatGPT" {
                    Some(EnvironmentPlatform {
                        os: "macos".into(),
                        architecture: "any".into(),
                    })
                } else {
                    None
                },
                install_scope: EnvironmentInstallScope::Project,
                install_source: (!gui).then(|| "mise".into()),
                detect_only: gui,
            },
        );
    }
    fn secret(&mut self, file: &EvidenceFile, name: &str, line: Option<u32>) {
        if !environment_variable_name(name) || project_environment_protected_name(name) {
            return;
        }
        self.add(
            file,
            &format!("secret:{name}"),
            name,
            line,
            RequirementSpec::Secrets {
                name: name.into(),
                vault: None,
            },
        );
    }
    fn tool(&mut self, file: &EvidenceFile, kind: AgentToolKind, name: &str, line: Option<u32>) {
        if !safe_metadata(name) {
            return;
        }
        let scope = RequirementScope::Folder {
            folder_id: file.folder_id.clone(),
        };
        // A file-bound registry reference is a proposal, never an admitted MCP/skill.
        self.add(
            file,
            &format!("tool:{name}"),
            name,
            line,
            RequirementSpec::AgentTools {
                tool_kind: kind,
                registry_ref: format!("{}:{}:{name}", file.folder_id, file.path),
                package_digest: file.digest.clone(),
                scope,
                runtime_requirements: vec![],
                vault_refs: vec![],
                compatible_providers: vec![],
            },
        );
    }
    fn source_steps(&mut self, file: &EvidenceFile, title: &str, line: Option<u32>) {
        // Retain source identity; opaque commands are not projected before review.
        self.add(
            file,
            "setup",
            title,
            line,
            RequirementSpec::SetupChecks {
                commands: vec![],
                probes: vec![],
                boundary: EnvironmentExecutionBoundary::DisposableWorker,
                source_path: Some(file.path.clone()),
                source_digest: file.digest.clone(),
                legacy: None,
            },
        );
    }
    pub fn import_file(&mut self, file: &EvidenceFile) {
        // Code already travels as source. Only protected declarations need a separate
        // transfer review; plain-folder documents/assets remain file proposals.
        let name = std::path::Path::new(&file.path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");
        if self.code_folders.contains(&file.folder_id)
            && !name.starts_with(".env")
            && !credential_configuration_name(name)
        {
            return;
        }
        self.add(
            file,
            "file",
            &file.path,
            None,
            RequirementSpec::Files {
                folder_id: file.folder_id.clone(),
                entries: vec![EnvironmentFile {
                    relative_path: file.path.clone(),
                    kind: EnvironmentFileKind::File,
                    content_digest: file.digest.clone(),
                    folder_id: file.folder_id.clone(),
                    user_selected: false,
                    git_ignored: None,
                    byte_count: file.bytes,
                    credential_filter_verdict: if file.protected {
                        EnvironmentCredentialFilterVerdict::NeedsVault
                    } else if file.text.is_some() {
                        EnvironmentCredentialFilterVerdict::Clear
                    } else {
                        EnvironmentCredentialFilterVerdict::NotChecked
                    },
                    transfer_inclusion: if file.protected || file.text.is_none() {
                        EnvironmentTransferInclusion::Exclude
                    } else {
                        EnvironmentTransferInclusion::ReviewRequired
                    },
                    reason: Some(
                        if file.protected {
                            "Protected input · choose Vault"
                        } else {
                            "Detected file · review required"
                        }
                        .into(),
                    ),
                    secret_looking: file.protected,
                }],
            },
        );
    }
    pub fn import(&mut self, file: &EvidenceFile) {
        let name = std::path::Path::new(&file.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        for gui in ["Canva", "Notion", "ChatGPT"] {
            if contains_word(&name.to_ascii_lowercase(), &gui.to_ascii_lowercase()) {
                self.software(file, gui, None, None, true)
            }
        }
        let Some(text) = &file.text else { return };
        if !file.protected {
            for gui in ["Canva", "Notion", "ChatGPT"] {
                if let Some((number, _)) = text
                    .lines()
                    .enumerate()
                    .find(|(_, line)| contains_word(line, gui))
                {
                    self.software(file, gui, None, Some(number as u32 + 1), true)
                }
            }
        }
        let is_example = name.starts_with(".env")
            && [".example", ".sample", ".template"]
                .iter()
                .any(|s| name.ends_with(s));
        for (n, line) in text.lines().enumerate() {
            if is_example {
                if let Some((key, _)) = line
                    .trim()
                    .strip_prefix("export ")
                    .unwrap_or(line.trim())
                    .split_once('=')
                {
                    self.secret(file, key.trim(), Some(n as u32 + 1))
                }
            }
            if !is_example && !credential_configuration_name(name) {
                for key in super::index::referenced_names(line) {
                    self.secret(file, &key, Some(n as u32 + 1))
                }
            }
        }
        match name {
            ".nvmrc" | ".python-version" => {
                let version = text.trim();
                let line = version_token(version)
                    .then(|| text.lines().position(|line| !line.trim().is_empty()))
                    .flatten()
                    .map(|n| n as u32 + 1);
                self.software(
                    file,
                    if name == ".nvmrc" { "node" } else { "python" },
                    Some(version),
                    line,
                    false,
                )
            }
            ".tool-versions" => {
                for (n, line) in text.lines().enumerate() {
                    let mut parts = line.split_whitespace();
                    if let (Some(tool), Some(version)) = (parts.next(), parts.next()) {
                        self.software(
                            file,
                            if tool == "nodejs" { "node" } else { tool },
                            Some(version),
                            Some(n as u32 + 1),
                            false,
                        )
                    }
                }
            }
            "mise.toml" | ".mise.toml" => {
                if let Ok(value) = text.parse::<toml::Value>() {
                    if let Some(tools) = value.get("tools").and_then(toml::Value::as_table) {
                        for (tool, value) in tools {
                            self.software(
                                file,
                                tool,
                                value.as_str(),
                                unique_line(text, tool),
                                false,
                            )
                        }
                    }
                    if value.get("tasks").is_some() || value.get("hooks").is_some() {
                        self.source_steps(file, "mise tasks/hooks · review only", None)
                    }
                }
            }
            "package.json" => {
                if let Ok(value) = serde_json::from_str::<Value>(text) {
                    self.code_folders.insert(file.folder_id.clone());
                    self.software(
                        file,
                        "node",
                        value.pointer("/engines/node").and_then(Value::as_str),
                        unique_line(text, "node"),
                        false,
                    );
                    if let Some(manager) = value.get("packageManager").and_then(Value::as_str) {
                        let (name, version) = manager.split_once('@').unwrap_or((manager, ""));
                        self.software(
                            file,
                            name,
                            Some(version),
                            unique_line(text, "packageManager"),
                            false,
                        );
                    }
                    for key in [
                        "dependencies",
                        "devDependencies",
                        "optionalDependencies",
                        "peerDependencies",
                    ] {
                        if let Some(deps) = value.get(key).and_then(Value::as_object) {
                            for (name, v) in deps.iter().take(512) {
                                if identity(name) {
                                    self.add(
                                        file,
                                        &format!("package:{name}"),
                                        name,
                                        unique_line(text, name),
                                        RequirementSpec::Software {
                                            identity: name.clone(),
                                            version_constraint: v
                                                .as_str()
                                                .filter(|v| version_token(v))
                                                .map(str::to_owned),
                                            platform: None,
                                            install_scope: EnvironmentInstallScope::Project,
                                            install_source: Some("npm package".into()),
                                            detect_only: false,
                                        },
                                    );
                                }
                            }
                        }
                    }
                    if value.get("scripts").is_some() {
                        self.source_steps(
                            file,
                            "Package scripts · review only",
                            unique_line(text, "scripts"),
                        )
                    }
                }
            }
            "Cargo.toml" | "pyproject.toml" | "Gemfile" | "go.mod" | "composer.json"
            | "pom.xml" | "build.gradle" | "requirements.txt" | "mix.exs" | "Package.swift"
            | "CMakeLists.txt" | "pubspec.yaml" | "deno.json" | "deno.jsonc" => {
                self.code_folders.insert(file.folder_id.clone());
                let runtime = match name {
                    "Cargo.toml" => "rust",
                    "pyproject.toml" | "requirements.txt" => "python",
                    "Gemfile" => "ruby",
                    "go.mod" => "go",
                    "composer.json" => "php",
                    "pom.xml" | "build.gradle" => "java",
                    "mix.exs" => "elixir",
                    "Package.swift" => "swift",
                    "pubspec.yaml" => "dart",
                    "deno.json" | "deno.jsonc" => "deno",
                    _ => "cmake",
                };
                self.software(file, runtime, None, None, false);
                self.source_steps(file, "Manifest setup/checks · review only", None);
            }
            "pnpm-lock.yaml" | "yarn.lock" | "package-lock.json" | "bun.lock" | "bun.lockb" => {
                self.software(
                    file,
                    match name {
                        "pnpm-lock.yaml" => "pnpm",
                        "yarn.lock" => "yarn",
                        "bun.lock" | "bun.lockb" => "bun",
                        _ => "npm",
                    },
                    None,
                    None,
                    false,
                );
                self.source_steps(file, "Locked dependencies · review only", None);
            }
            "Cargo.lock" | "poetry.lock" | "uv.lock" | "Gemfile.lock" | "go.sum"
            | "composer.lock" => self.source_steps(file, "Locked dependencies · review only", None),
            "devcontainer.json" => match parse_jsonc(text) {
                Some(value) => {
                    if let Some(image) = value
                        .get("image")
                        .and_then(Value::as_str)
                        .filter(|v| identity(v))
                    {
                        self.add(
                            file,
                            "devcontainer-image",
                            "Devcontainer image · import only",
                            unique_line(text, "image"),
                            RequirementSpec::Software {
                                identity: image.into(),
                                version_constraint: None,
                                platform: None,
                                install_scope: EnvironmentInstallScope::Project,
                                install_source: Some("devcontainer".into()),
                                detect_only: true,
                            },
                        );
                    }
                    if let Some(features) = value.get("features").and_then(Value::as_object) {
                        for feature in features.keys().filter(|v| identity(v)) {
                            self.software(file, feature, None, unique_line(text, feature), true)
                        }
                    }
                    for key in ["containerEnv", "remoteEnv"] {
                        if let Some(values) = value.get(key).and_then(Value::as_object) {
                            for name in values.keys() {
                                self.secret(file, name, unique_line(text, name))
                            }
                        }
                    }
                    if [
                        "initializeCommand",
                        "onCreateCommand",
                        "updateContentCommand",
                        "postCreateCommand",
                        "postStartCommand",
                        "postAttachCommand",
                    ]
                    .iter()
                    .any(|key| value.get(*key).is_some())
                    {
                        self.source_steps(file, "Devcontainer lifecycle · review only", None)
                    }
                    for key in value
                        .as_object()
                        .into_iter()
                        .flat_map(|v| v.keys())
                        .filter(|key| {
                            ![
                                "name",
                                "image",
                                "features",
                                "containerEnv",
                                "remoteEnv",
                                "initializeCommand",
                                "onCreateCommand",
                                "updateContentCommand",
                                "postCreateCommand",
                                "postStartCommand",
                                "postAttachCommand",
                            ]
                            .contains(&key.as_str())
                        })
                    {
                        self.skips.push(skip(
                            &file.folder_id,
                            &file.path,
                            if key == "mounts" {
                                "devcontainer_mounts_review_required"
                            } else {
                                "unsupported_devcontainer_field"
                            },
                        ));
                    }
                }
                None => self
                    .skips
                    .push(skip(&file.folder_id, &file.path, "invalid_devcontainer")),
            },
            "compose.yaml" | "compose.yml" | "docker-compose.yml" | "docker-compose.yaml" => {
                if let Ok(value) = serde_yaml::from_str::<serde_yaml::Value>(text) {
                    if let Some(services) = value
                        .get("services")
                        .and_then(serde_yaml::Value::as_mapping)
                    {
                        for (name, service) in services {
                            let Some(name) = name.as_str().filter(|n| identity(n)) else {
                                continue;
                            };
                            self.add(
                                file,
                                &format!("service:{name}"),
                                name,
                                unique_line(text, name),
                                RequirementSpec::Services {
                                    identity: name.into(),
                                    target_binding: None,
                                    probe: None,
                                    health_expectation: Some(
                                        EnvironmentHealthExpectation::Reachable,
                                    ),
                                    depends_on: vec![],
                                },
                            );
                            if service.get("healthcheck").is_some() {
                                self.source_steps(
                                    file,
                                    "Compose health checks · review only",
                                    unique_line(text, "healthcheck"),
                                )
                            }
                            if let Some(env) = service.get("environment") {
                                if let Some(mapping) = env.as_mapping() {
                                    for key in mapping.keys().filter_map(serde_yaml::Value::as_str)
                                    {
                                        self.secret(file, key, unique_line(text, key))
                                    }
                                }
                                if let Some(sequence) = env.as_sequence() {
                                    for item in
                                        sequence.iter().filter_map(serde_yaml::Value::as_str)
                                    {
                                        self.secret(
                                            file,
                                            item.split('=').next().unwrap_or(""),
                                            unique_line(text, item.split('=').next().unwrap_or("")),
                                        )
                                    }
                                }
                            }
                            if ["command", "privileged", "ports", "volumes", "build"]
                                .iter()
                                .any(|key| service.get(*key).is_some())
                            {
                                self.skips.push(skip(
                                    &file.folder_id,
                                    &file.path,
                                    "compose_execution_or_exposure_review_required",
                                ));
                            }
                        }
                    }
                }
            }
            ".mcp.json" => {
                if let Ok(value) = serde_json::from_str::<Value>(text) {
                    if let Some(servers) = value.get("mcpServers").and_then(Value::as_object) {
                        for (name, server) in servers {
                            self.tool(file, AgentToolKind::Mcp, name, unique_line(text, name));
                            if let Some(env) = server.get("env").and_then(Value::as_object) {
                                for key in env.keys() {
                                    self.secret(file, key, unique_line(text, key))
                                }
                            }
                        }
                    }
                }
            }
            "AGENTS.md" | "CLAUDE.md" | "GEMINI.md" => self.tool(
                file,
                AgentToolKind::InstructionFile,
                name,
                text.lines().next().map(|_| 1),
            ),
            "SKILL.md" => self.tool(
                file,
                AgentToolKind::Skill,
                &file.path,
                text.lines().next().map(|_| 1),
            ),
            _ => {}
        }
        // Structured dependency declarations are language-independent evidence.
        // A coding manifest need not belong to the runtime adapters listed above.
        let document = if name.ends_with(".json") {
            serde_json::from_str::<Value>(text).ok()
        } else if name.ends_with(".toml") {
            text.parse::<toml::Value>()
                .ok()
                .and_then(|v| serde_json::to_value(v).ok())
        } else {
            None
        };
        if let Some(document) = document {
            for key in ["dependencies", "dev-dependencies", "build-dependencies"] {
                if let Some(dependencies) = document.get(key).and_then(Value::as_object) {
                    for (name, value) in dependencies.iter().take(512) {
                        let version = value
                            .as_str()
                            .or_else(|| value.get("version").and_then(Value::as_str));
                        if !identity(name) || !version.is_some_and(version_token) {
                            continue;
                        }
                        self.code_folders.insert(file.folder_id.clone());
                        self.add(
                            file,
                            &format!("package:{name}"),
                            name,
                            unique_line(text, name),
                            RequirementSpec::Software {
                                identity: name.clone(),
                                version_constraint: version.map(str::to_owned),
                                platform: None,
                                install_scope: EnvironmentInstallScope::Project,
                                install_source: Some("manifest package".into()),
                                detect_only: false,
                            },
                        );
                    }
                }
            }
            if let Some(version) = document
                .pointer("/package/rust-version")
                .and_then(Value::as_str)
            {
                self.software(
                    file,
                    "rust",
                    Some(version),
                    unique_line(text, "rust-version"),
                    false,
                );
            }
            if let Some(version) = document
                .pointer("/project/requires-python")
                .and_then(Value::as_str)
            {
                self.software(
                    file,
                    "python",
                    Some(version),
                    unique_line(text, "requires-python"),
                    false,
                );
            }
        }
    }
}
fn identity(value: &str) -> bool {
    safe_metadata(value)
        && value.len() <= 256
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@_./:-+".contains(&b))
}
fn version_token(value: &str) -> bool {
    safe_metadata(value)
        && value.len() <= 128
        && value.bytes().any(|b| b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || b"v.xX*<>=~^| ,-+".contains(&b))
}
fn unique_line(text: &str, key: &str) -> Option<u32> {
    let mut matches = text.lines().enumerate().filter(|(_, line)| {
        line.contains(&format!("\"{key}\""))
            || line.trim_start().starts_with(&format!("{key}:"))
            || line.trim_start().starts_with(&format!("{key} ="))
    });
    let line = matches.next().map(|(n, _)| n as u32 + 1);
    if matches.next().is_some() {
        None
    } else {
        line
    }
}
/// JSON comments/trailing commas are lexical data; quoted URLs and escaped quotes stay intact.
fn parse_jsonc(text: &str) -> Option<Value> {
    let mut output = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut quoted = false;
    let mut escaped = false;
    while let Some(c) = chars.next() {
        if quoted {
            output.push(c);
            if escaped {
                escaped = false
            } else if c == '\\' {
                escaped = true
            } else if c == '"' {
                quoted = false
            }
            continue;
        }
        if c == '"' {
            quoted = true;
            output.push(c);
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for c in chars.by_ref() {
                if c == '\n' {
                    output.push(c);
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '\n' {
                    output.push(c)
                }
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        if c == ',' {
            let mut ahead = chars.clone();
            if ahead
                .find(|c| !c.is_whitespace())
                .is_some_and(|c| c == '}' || c == ']')
            {
                continue;
            }
        }
        output.push(c);
    }
    serde_json::from_str(&output).ok()
}

// MP-08 / MP-10 / MP-11: A GUI name is a complete word, never Canvas or Notional.
fn contains_word(source: &str, word: &str) -> bool {
    source
        .split(|character: char| !character.is_alphanumeric())
        .any(|token| token == word)
}
