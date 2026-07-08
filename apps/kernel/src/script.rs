use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::DaemonError;
use crate::mcp::validate_registry_name;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

mod execution;

use execution::{execute_node_script, execute_python_script, inspect_script};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaEnvironmentConfig {
    pub name: String,
    pub runtime: ArrobaEnvironmentRuntime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ArrobaEnvironmentRuntime {
    Python {
        python: PathBuf,
    },
    Node {
        node: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        package_root: Option<PathBuf>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrobaScriptMetadata {
    pub name: String,
    pub runtime: ArrobaScriptRuntime,
    pub path: PathBuf,
    pub description: String,
    pub input_schema: Value,
    pub definition_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_sec: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrobaScriptRuntime {
    Python,
    #[serde(rename = "typescript")]
    TypeScript,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrobaEnvironmentRegistry {
    roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArrobaScriptRegistry {
    roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredScriptMetadata {
    name: String,
    runtime: ArrobaScriptRuntime,
    entrypoint: String,
    description: String,
    input_schema: Value,
    definition_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout_sec: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptExecutionResult {
    pub ok: bool,
    pub payload: Value,
    pub logs: String,
}
pub const DEFAULT_SCRIPT_EXECUTION_TIMEOUT_SEC: u64 = 30;
impl ArrobaEnvironmentRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        capability_root_for("envs", workspace.as_ref())
    }

    pub fn user_root() -> Option<PathBuf> {
        user_capability_root_for("envs")
    }

    pub fn install(&self, config: &ArrobaEnvironmentConfig) -> Result<PathBuf, DaemonError> {
        config.validate()?;
        let root = self.primary_root()?;
        fs::create_dir_all(root).map_err(io_error("env.install"))?;
        let path = root.join(format!("{}.json", config.name));
        let payload =
            serde_json::to_string_pretty(config).map_err(|error| DaemonError::LocalTransport {
                operation: "env.install",
                message: format!("failed to serialize environment `{}`: {error}", config.name),
            })?;
        fs::write(&path, format!("{payload}\n")).map_err(io_error("env.install"))?;
        Ok(path)
    }

    pub fn uninstall(&self, name: &str) -> Result<PathBuf, DaemonError> {
        validate_registry_name(name, "environment name")?;
        let path = self
            .find_path(name)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "env.uninstall",
                message: format!("environment `{name}` is not registered"),
            })?;
        fs::remove_file(&path).map_err(io_error("env.uninstall"))?;
        Ok(path)
    }

    pub fn list(&self) -> Result<Vec<ArrobaEnvironmentConfig>, DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            for entry in fs::read_dir(root).map_err(io_error("env.list"))? {
                let path = entry.map_err(io_error("env.list"))?.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let config = Self::read(&path)?;
                entries.entry(config.name.clone()).or_insert(config);
            }
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<ArrobaEnvironmentConfig>, DaemonError> {
        validate_registry_name(name, "environment name")?;
        let Some(path) = self.find_path(name)? else {
            return Ok(None);
        };
        Self::read(&path).map(Some)
    }

    fn read(path: &Path) -> Result<ArrobaEnvironmentConfig, DaemonError> {
        let contents = fs::read_to_string(path).map_err(io_error("env.read"))?;
        serde_json::from_str(&contents).map_err(|error| DaemonError::LocalTransport {
            operation: "env.read",
            message: format!("failed to parse environment `{}`: {error}", path.display()),
        })
    }

    fn find_path(&self, name: &str) -> Result<Option<PathBuf>, DaemonError> {
        for root in &self.roots {
            let path = root.join(format!("{name}.json"));
            if path.exists() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn primary_root(&self) -> Result<&PathBuf, DaemonError> {
        self.roots.first().ok_or(DaemonError::InvalidConfig {
            field: "environment registry roots",
            message: "must include at least one root",
        })
    }
}

impl ArrobaEnvironmentConfig {
    pub fn validate(&self) -> Result<(), DaemonError> {
        validate_registry_name(&self.name, "environment name")?;
        match &self.runtime {
            ArrobaEnvironmentRuntime::Python { python } => {
                if !python.exists() {
                    return Err(DaemonError::InvalidConfig {
                        field: "python",
                        message: "python executable does not exist",
                    });
                }
                let status = Command::new(python)
                    .arg("--version")
                    .status()
                    .map_err(io_error("env.validate"))?;
                if !status.success() {
                    return Err(DaemonError::InvalidConfig {
                        field: "python",
                        message: "python executable failed --version",
                    });
                }
            }
            ArrobaEnvironmentRuntime::Node { node, package_root } => {
                if !node.exists() {
                    return Err(DaemonError::InvalidConfig {
                        field: "node",
                        message: "node executable does not exist",
                    });
                }
                if let Some(package_root) = package_root {
                    if !package_root.exists() {
                        return Err(DaemonError::InvalidConfig {
                            field: "package_root",
                            message: "node package root does not exist",
                        });
                    }
                }
                let status = Command::new(node)
                    .arg("--version")
                    .status()
                    .map_err(io_error("env.validate"))?;
                if !status.success() {
                    return Err(DaemonError::InvalidConfig {
                        field: "node",
                        message: "node executable failed --version",
                    });
                }
            }
        }
        Ok(())
    }
}

impl ArrobaScriptRegistry {
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self { roots }
    }

    pub fn project_root(workspace: impl AsRef<Path>) -> PathBuf {
        capability_root_for("scripts", workspace.as_ref())
    }

    pub fn user_root() -> Option<PathBuf> {
        user_capability_root_for("scripts")
    }

    pub fn validate_script(
        &self,
        source: &Path,
        name: Option<&str>,
        env: &ArrobaEnvironmentConfig,
    ) -> Result<ArrobaScriptMetadata, DaemonError> {
        let name = name.map(str::to_string).unwrap_or_else(|| {
            source
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
        validate_registry_name(&name, "script name")?;
        if !source.exists() || !source.is_file() {
            return Err(DaemonError::InvalidConfig {
                field: "script path",
                message: "script path must be a file",
            });
        }
        let runtime = runtime_for_path(source)?;
        match (&runtime, &env.runtime) {
            (ArrobaScriptRuntime::Python, ArrobaEnvironmentRuntime::Python { .. }) => {}
            (ArrobaScriptRuntime::TypeScript, ArrobaEnvironmentRuntime::Node { .. }) => {}
            _ => {
                return Err(DaemonError::InvalidConfig {
                    field: "environment",
                    message: "script runtime and environment runtime do not match",
                });
            }
        }
        let (description, input_schema) = inspect_script(source, &runtime, env)?;
        let definition_hash = script_definition_hash(source, &description, &input_schema)?;
        Ok(ArrobaScriptMetadata {
            name,
            runtime,
            path: source.to_path_buf(),
            description,
            input_schema,
            definition_hash,
            timeout_sec: None,
        })
    }

    pub fn install(
        &self,
        source: &Path,
        name: Option<&str>,
        env: &ArrobaEnvironmentConfig,
    ) -> Result<(ArrobaScriptMetadata, PathBuf), DaemonError> {
        let validated = self.validate_script(source, name, env)?;
        let root = self.primary_root()?;
        let destination = root.join(&validated.name);
        if destination.exists() {
            return Err(DaemonError::LocalTransport {
                operation: "script.install",
                message: format!("script `{}` is already registered", validated.name),
            });
        }
        fs::create_dir_all(&destination).map_err(io_error("script.install"))?;
        let entrypoint = match validated.runtime {
            ArrobaScriptRuntime::Python => "script.py",
            ArrobaScriptRuntime::TypeScript => "script.ts",
        };
        let script_path = destination.join(entrypoint);
        fs::copy(source, &script_path).map_err(io_error("script.install"))?;
        let stored = StoredScriptMetadata {
            name: validated.name.clone(),
            runtime: validated.runtime.clone(),
            entrypoint: entrypoint.to_string(),
            description: validated.description.clone(),
            input_schema: validated.input_schema.clone(),
            definition_hash: validated.definition_hash.clone(),
            timeout_sec: validated.timeout_sec,
        };
        fs::write(
            destination.join("metadata.json"),
            format!(
                "{}\n",
                serde_json::to_string_pretty(&stored).map_err(|error| {
                    DaemonError::LocalTransport {
                        operation: "script.install",
                        message: format!("failed to serialize script metadata: {error}"),
                    }
                })?
            ),
        )
        .map_err(io_error("script.install"))?;
        Ok((self.metadata_from_stored(&destination, stored), destination))
    }

    pub fn uninstall(&self, name: &str) -> Result<(ArrobaScriptMetadata, PathBuf), DaemonError> {
        let dir = self
            .find_dir(name)?
            .ok_or_else(|| DaemonError::LocalTransport {
                operation: "script.uninstall",
                message: format!("script `{name}` is not registered"),
            })?;
        let metadata = self.read_metadata(&dir)?;
        fs::remove_dir_all(&dir).map_err(io_error("script.uninstall"))?;
        Ok((metadata, dir))
    }

    pub fn list(&self) -> Result<Vec<ArrobaScriptMetadata>, DaemonError> {
        let mut entries = BTreeMap::new();
        for root in &self.roots {
            if !root.exists() {
                continue;
            }
            for entry in fs::read_dir(root).map_err(io_error("script.list"))? {
                let path = entry.map_err(io_error("script.list"))?.path();
                if !path.is_dir() || !path.join("metadata.json").exists() {
                    continue;
                }
                let metadata = self.read_metadata(&path)?;
                entries.entry(metadata.name.clone()).or_insert(metadata);
            }
        }
        Ok(entries.into_values().collect())
    }

    pub fn get(&self, name: &str) -> Result<Option<ArrobaScriptMetadata>, DaemonError> {
        let Some(dir) = self.find_dir(name)? else {
            return Ok(None);
        };
        self.read_metadata(&dir).map(Some)
    }

    pub fn execute(
        &self,
        name: &str,
        env: &ArrobaEnvironmentConfig,
        arguments: Value,
    ) -> Result<ScriptExecutionResult, DaemonError> {
        let metadata = self.get(name)?.ok_or_else(|| DaemonError::LocalTransport {
            operation: "script.execute",
            message: format!("script `{name}` is not registered"),
        })?;
        match (&metadata.runtime, &env.runtime) {
            (ArrobaScriptRuntime::Python, ArrobaEnvironmentRuntime::Python { python }) => {
                execute_python_script(
                    python,
                    &metadata.path,
                    arguments,
                    metadata
                        .timeout_sec
                        .unwrap_or(DEFAULT_SCRIPT_EXECUTION_TIMEOUT_SEC),
                )
            }
            (
                ArrobaScriptRuntime::TypeScript,
                ArrobaEnvironmentRuntime::Node { node, package_root },
            ) => execute_node_script(
                node,
                package_root.as_deref(),
                &metadata.path,
                &metadata.input_schema,
                arguments,
                metadata
                    .timeout_sec
                    .unwrap_or(DEFAULT_SCRIPT_EXECUTION_TIMEOUT_SEC),
            ),
            _ => Err(DaemonError::InvalidConfig {
                field: "environment",
                message: "script runtime and environment runtime do not match",
            }),
        }
    }

    fn read_metadata(&self, dir: &Path) -> Result<ArrobaScriptMetadata, DaemonError> {
        let contents =
            fs::read_to_string(dir.join("metadata.json")).map_err(io_error("script.read"))?;
        let stored = serde_json::from_str::<StoredScriptMetadata>(&contents).map_err(|error| {
            DaemonError::LocalTransport {
                operation: "script.read",
                message: format!(
                    "failed to parse script metadata `{}`: {error}",
                    dir.display()
                ),
            }
        })?;
        Ok(self.metadata_from_stored(dir, stored))
    }

    fn metadata_from_stored(
        &self,
        dir: &Path,
        stored: StoredScriptMetadata,
    ) -> ArrobaScriptMetadata {
        ArrobaScriptMetadata {
            name: stored.name,
            runtime: stored.runtime,
            path: dir.join(stored.entrypoint),
            description: stored.description,
            input_schema: stored.input_schema,
            definition_hash: stored.definition_hash,
            timeout_sec: stored.timeout_sec,
        }
    }

    fn find_dir(&self, name: &str) -> Result<Option<PathBuf>, DaemonError> {
        validate_registry_name(name, "script name")?;
        for root in &self.roots {
            let path = root.join(name);
            if path.join("metadata.json").exists() {
                return Ok(Some(path));
            }
        }
        Ok(None)
    }

    fn primary_root(&self) -> Result<&PathBuf, DaemonError> {
        self.roots.first().ok_or(DaemonError::InvalidConfig {
            field: "script registry roots",
            message: "must include at least one root",
        })
    }
}

fn runtime_for_path(path: &Path) -> Result<ArrobaScriptRuntime, DaemonError> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py") => Ok(ArrobaScriptRuntime::Python),
        Some("ts") => Ok(ArrobaScriptRuntime::TypeScript),
        _ => Err(DaemonError::InvalidConfig {
            field: "script path",
            message: "script extension must be .py or .ts",
        }),
    }
}

fn script_definition_hash(
    path: &Path,
    description: &str,
    input_schema: &Value,
) -> Result<String, DaemonError> {
    let contents = fs::read(path).map_err(io_error("script.hash"))?;
    let mut hasher = Sha256::new();
    hasher.update(contents);
    hasher.update(description.as_bytes());
    hasher.update(input_schema.to_string().as_bytes());
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn capability_root_for(kind: &str, workspace: &Path) -> PathBuf {
    if let Some(root) = managed_capability_root() {
        return root
            .join("project")
            .join(workspace_registry_hash(workspace))
            .join(kind);
    }
    workspace.join(".arroba").join(kind)
}

fn user_capability_root_for(kind: &str) -> Option<PathBuf> {
    if let Some(root) = managed_capability_root() {
        return Some(root.join("user").join(kind));
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join(".arroba").join(kind))
}

fn managed_capability_root() -> Option<PathBuf> {
    std::env::var_os("ARROBA_CAPABILITY_ISOLATION_ROOT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn workspace_registry_hash(workspace: &Path) -> String {
    let digest = Sha256::digest(workspace.to_string_lossy().as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn io_error(operation: &'static str) -> impl Fn(std::io::Error) -> DaemonError {
    move |error| DaemonError::LocalTransport {
        operation,
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn python_script_validates_installs_and_executes() {
        let Some(python) = find_existing(&[
            std::env::var_os("PYTHON").map(PathBuf::from),
            Some(PathBuf::from("/usr/bin/python3")),
            Some(PathBuf::from("/opt/homebrew/bin/python3")),
            Some(PathBuf::from("/usr/local/bin/python3")),
        ]) else {
            return;
        };
        let root = temp_root("arroba-script-python");
        let source = root.join("lookup.py");
        fs::create_dir_all(&root).expect("temp root should be created");
        fs::write(
            &source,
            r#"
def run(key: str, limit: int = 1) -> list[int]:
    """Return deterministic row ids from a local fixture."""
    print("script-log")
    return list(range(limit))

def test_run() -> None:
    result = run("demo", 2)
    assert result == [0, 1]
"#,
        )
        .expect("script should be written");
        let env = ArrobaEnvironmentConfig {
            name: "py".to_string(),
            runtime: ArrobaEnvironmentRuntime::Python { python },
        };
        let registry = ArrobaScriptRegistry::new(vec![root.join("scripts")]);

        let validated = registry
            .validate_script(&source, Some("lookup"), &env)
            .expect("script should validate");
        assert_eq!(validated.name, "lookup");
        assert_eq!(validated.runtime, ArrobaScriptRuntime::Python);
        assert_eq!(
            validated.input_schema["properties"]["key"]["type"],
            "string"
        );
        assert_eq!(
            validated.input_schema["properties"]["limit"]["type"],
            "integer"
        );

        registry
            .install(&source, Some("lookup"), &env)
            .expect("script should install");
        let result = registry
            .execute(
                "lookup",
                &env,
                serde_json::json!({
                    "key": "abc",
                    "limit": 3
                }),
            )
            .expect("script should execute");
        assert!(result.ok);
        assert_eq!(result.payload, serde_json::json!([0, 1, 2]));
        assert!(result.logs.contains("script-log"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn python_script_validation_rejects_missing_type_hints() {
        let Some(python) = find_existing(&[
            std::env::var_os("PYTHON").map(PathBuf::from),
            Some(PathBuf::from("/usr/bin/python3")),
            Some(PathBuf::from("/opt/homebrew/bin/python3")),
            Some(PathBuf::from("/usr/local/bin/python3")),
        ]) else {
            return;
        };
        let root = temp_root("arroba-script-invalid");
        let source = root.join("bad.py");
        fs::create_dir_all(&root).expect("temp root should be created");
        fs::write(
            &source,
            r#"
def run(key) -> list[int]:
    """Bad script."""
    return [1]

def test_run() -> None:
    run("demo")
"#,
        )
        .expect("script should be written");
        let env = ArrobaEnvironmentConfig {
            name: "py".to_string(),
            runtime: ArrobaEnvironmentRuntime::Python { python },
        };
        let registry = ArrobaScriptRegistry::new(vec![root.join("scripts")]);

        let error = registry
            .validate_script(&source, Some("bad"), &env)
            .expect_err("script should be rejected");
        assert!(error.to_string().contains("type annotation"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn typescript_script_validates_and_executes_when_tsx_is_available() {
        let Some(node) = find_existing(&[
            std::env::var_os("NODE").map(PathBuf::from),
            Some(PathBuf::from("/opt/homebrew/bin/node")),
            Some(PathBuf::from("/usr/local/bin/node")),
            Some(PathBuf::from("/usr/bin/node")),
        ]) else {
            return;
        };
        if !Command::new(&node)
            .arg("--import")
            .arg("tsx")
            .arg("-e")
            .arg("console.log('ok')")
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            return;
        }
        let root = temp_root("arroba-script-typescript");
        let source = root.join("vector_lookup.ts");
        fs::create_dir_all(&root).expect("temp root should be created");
        fs::write(
            &source,
            r#"
/**
 * Lookup a deterministic vector database record.
 */
export function run(query: string, limit: number = 1): Record<string, unknown> {
  console.log("ts-script-log")
  return { query, limit, ids: [1, 2, 3] }
}

export function test_run(): void {
  const result = run("demo", 2)
  if (result.query !== "demo") throw new Error("bad query")
}
"#,
        )
        .expect("script should be written");
        let env = ArrobaEnvironmentConfig {
            name: "node".to_string(),
            runtime: ArrobaEnvironmentRuntime::Node {
                node,
                package_root: None,
            },
        };
        let registry = ArrobaScriptRegistry::new(vec![root.join("scripts")]);

        registry
            .install(&source, Some("vector_lookup"), &env)
            .expect("script should install");
        let result = registry
            .execute(
                "vector_lookup",
                &env,
                serde_json::json!({
                    "query": "invoice",
                    "limit": 2
                }),
            )
            .expect("script should execute");
        assert!(result.ok);
        assert_eq!(result.payload["query"], "invoice");
        assert_eq!(result.payload["limit"], 2);
        assert_eq!(result.payload["ids"], serde_json::json!([1, 2, 3]));
        assert!(result.logs.contains("ts-script-log"));

        let _ = fs::remove_dir_all(root);
    }

    fn find_existing(candidates: &[Option<PathBuf>]) -> Option<PathBuf> {
        candidates
            .iter()
            .flatten()
            .find(|candidate| candidate.exists())
            .cloned()
    }

    fn temp_root(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nanos}"))
    }
}
