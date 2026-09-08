use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use jsonschema::{Draft, JSONSchema, SchemaResolver, SchemaResolverError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    json::read_json,
    manifest::{valid_function_name, validate_origin},
    ErrorCode, HttpMethod, Limits, Manifest, PackageError, Result,
};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Declarations {
    #[serde(default)]
    pub tools: Vec<ToolDeclaration>,
    #[serde(default)]
    pub events: Vec<EventDeclaration>,
    #[serde(default)]
    pub actions: Vec<ActionDeclaration>,
    #[serde(default)]
    pub information_sets: Vec<InformationSet>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolDeclaration {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EventDeclaration {
    pub name: String,
    /// Publisher-signed payload schema version, matched by every occurrence.
    pub schema_version: u32,
    pub direction: EventDirection,
    pub payload_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter_schema: Option<Value>,
}

/// Signed authority separates notifications emitted by the App from deliveries
/// handled by the App. An outgoing-only event never requires an incoming handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventDirection {
    Outgoing,
    Incoming,
    Both,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionDeclaration {
    pub name: String,
    pub input_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical_validation: Option<CriticalValidation>,
    #[serde(default)]
    pub effect_routes: Vec<EffectRoute>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CriticalValidation {
    pub reason: String,
    pub user_verification: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EffectRoute {
    pub origin: String,
    pub method: HttpMethod,
    pub path: String,
    /// Symbolic connection class; never a credential or caller-selected handle.
    pub connection: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InformationSet {
    pub name: String,
    pub purpose: String,
    pub source_scope: InformationSourceScope,
    pub schema_version: u32,
    pub delivery: OutputDelivery,
    pub fields_schema: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validator: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InformationSourceScope {
    AppTask,
    AgentTurn,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputDelivery {
    Intermediate,
    Final,
    Both,
}

macro_rules! declaration_document {
    ($name:ident, $field:ident, $item:ty) => {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct $name {
            $field: Vec<$item>,
        }
    };
}
declaration_document!(ToolsDocument, tools, ToolDeclaration);
declaration_document!(EventsDocument, events, EventDeclaration);
declaration_document!(ActionsDocument, actions, ActionDeclaration);
declaration_document!(InformationDocument, information_sets, InformationSet);

pub(crate) fn read_declarations(
    manifest: &Manifest,
    files: &BTreeMap<String, &[u8]>,
    limits: &Limits,
) -> Result<Declarations> {
    fn load<T: serde::de::DeserializeOwned>(
        path: &str,
        files: &BTreeMap<String, &[u8]>,
        limits: &Limits,
    ) -> Result<T> {
        let bytes = files.get(path).ok_or_else(|| {
            PackageError::new(ErrorCode::MissingEntry, "missing declaration file")
        })?;
        read_json(
            bytes,
            limits.max_metadata_bytes,
            limits,
            ErrorCode::InvalidSchema,
            false,
        )
    }
    let mut declarations = Declarations::default();
    if let Some(path) = &manifest.tools {
        declarations.tools = load::<ToolsDocument>(path, files, limits)?.tools;
    }
    if let Some(path) = &manifest.events {
        declarations.events = load::<EventsDocument>(path, files, limits)?.events;
    }
    if let Some(path) = &manifest.actions {
        declarations.actions = load::<ActionsDocument>(path, files, limits)?.actions;
    }
    if let Some(path) = &manifest.information_sets {
        declarations.information_sets =
            load::<InformationDocument>(path, files, limits)?.information_sets;
    }
    declarations.validate(manifest, limits)?;
    Ok(declarations)
}

impl Declarations {
    pub fn validate(&self, manifest: &Manifest, limits: &Limits) -> Result<()> {
        check_names(self.tools.iter().map(|item| item.name.as_str()), limits)?;
        check_names(self.events.iter().map(|item| item.name.as_str()), limits)?;
        check_names(self.actions.iter().map(|item| item.name.as_str()), limits)?;
        check_names(
            self.information_sets.iter().map(|item| item.name.as_str()),
            limits,
        )?;
        for tool in &self.tools {
            validate_schema(&tool.input_schema, true, limits)?;
            if tool.description.len() > 4096 {
                return invalid("tool description exceeds limit");
            }
            if let Some(output) = &tool.output_schema {
                validate_schema(output, false, limits)?;
            }
            if let Some(action_name) = &tool.action {
                let action = self
                    .actions
                    .iter()
                    .find(|action| &action.name == action_name)
                    .ok_or_else(|| {
                        PackageError::new(
                            ErrorCode::InvalidSchema,
                            "tool references an undeclared action",
                        )
                    })?;
                if action.input_schema != tool.input_schema {
                    return invalid("tool and action input schemas must match");
                }
            }
        }
        for event in &self.events {
            if event.schema_version == 0 || manifest.min_kernel_protocol < 291 {
                return invalid(
                    "directed events require a positive schemaVersion and kernel protocol 291",
                );
            }
            validate_schema(&event.payload_schema, true, limits)?;
            if let Some(filter) = &event.filter_schema {
                validate_schema(filter, true, limits)?;
            }
        }
        let mut protected_routes = BTreeSet::new();
        for action in &self.actions {
            validate_schema(&action.input_schema, true, limits)?;
            if let Some(critical) = &action.critical_validation {
                if critical.reason.trim().is_empty()
                    || critical.reason.len() > 1024
                    || action.effect_routes.is_empty()
                {
                    return invalid(
                        "critical actions require a reason and protected effect routes",
                    );
                }
            }
            if action.effect_routes.len() > 64 {
                return invalid("too many effect routes");
            }
            for route in &action.effect_routes {
                validate_origin(&route.origin)?;
                if !crate::manifest::valid_identifier(&route.connection)
                    || !route.path.starts_with('/')
                    || route.path.starts_with("//")
                    || route.path.len() > 2048
                    || !route.path.is_ascii()
                    || route
                        .path
                        .bytes()
                        .any(|b| b <= 32 || b >= 127 || b"?#\\*%".contains(&b))
                    || route.path.split('/').any(|part| matches!(part, "." | ".."))
                {
                    return invalid(
                        "effect route must declare an exact path and symbolic connection",
                    );
                }
                if !manifest
                    .capabilities
                    .network
                    .iter()
                    .any(|dest| dest.origin == route.origin && dest.methods.contains(&route.method))
                {
                    return invalid("effect route is outside declared network capabilities");
                }
                // An origin/method/path cannot also be advertised through a
                // different, unprotected connection or action.
                if !protected_routes.insert((&route.origin, route.method, &route.path)) {
                    return invalid("effect route is declared by more than one action");
                }
            }
        }
        for info in &self.information_sets {
            if info.purpose.trim().is_empty()
                || info.purpose.len() > 1024
                || info.schema_version == 0
            {
                return invalid("information sets need a purpose and positive schema version");
            }
            validate_schema(&info.fields_schema, true, limits)?;
            if info
                .validator
                .as_ref()
                .is_some_and(|name| !valid_function_name(name))
            {
                return invalid("information validator must be a local function name");
            }
        }
        Ok(())
    }
}

/// Compile the actual Draft 7 schema, without network access. Schemas must be
/// bundled inline; remote IDs/references and recursive references are excluded
/// from v1 so package inspection cannot resolve URLs or recurse without bound.
pub fn validate_schema(schema: &Value, closed_object: bool, limits: &Limits) -> Result<()> {
    compile_schema(schema, closed_object, limits).map(|_| ())
}

/// Runtime consumers use this same compiler so their input checks cannot
/// resolve external resources or interpret annotation data as schema IDs.
pub fn compile_schema(schema: &Value, closed_object: bool, limits: &Limits) -> Result<JSONSchema> {
    crate::json::check_value(schema, limits)?;
    if serde_json::to_vec(schema)
        .map_err(|_| PackageError::new(ErrorCode::InvalidSchema, "invalid schema JSON"))?
        .len()
        > limits.max_schema_bytes
    {
        return Err(PackageError::new(
            ErrorCode::ArchiveLimit,
            "schema exceeds byte limit",
        ));
    }
    if closed_object
        && (schema.get("type").and_then(Value::as_str) != Some("object")
            || schema.get("additionalProperties") != Some(&Value::Bool(false)))
    {
        return invalid("input and information schemas must be closed objects");
    }
    let compilation_schema = schema_for_compilation(schema)?;
    JSONSchema::options()
        .with_draft(Draft::Draft7)
        .with_resolver(NoExternalSchemas)
        .compile(&compilation_schema)
        .map_err(|_| PackageError::new(ErrorCode::InvalidSchema, "invalid Draft 7 schema"))
}

struct NoExternalSchemas;

impl SchemaResolver for NoExternalSchemas {
    fn resolve(
        &self,
        _: &Value,
        _: &url::Url,
        _: &str,
    ) -> std::result::Result<Arc<Value>, SchemaResolverError> {
        Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "App schemas cannot resolve external resources",
        )
        .into())
    }
}

fn schema_for_compilation(value: &Value) -> Result<Value> {
    let Some(object) = value.as_object() else {
        return Ok(value.clone());
    };
    if ["$ref", "$id", "$dynamicRef", "$recursiveRef"]
        .iter()
        .any(|keyword| object.contains_key(*keyword))
    {
        return invalid("v1 schemas must be inline without IDs or references");
    }
    if object
        .get("$schema")
        .is_some_and(|schema| schema.as_str() != Some("http://json-schema.org/draft-07/schema#"))
    {
        return invalid("only JSON Schema Draft 7 is supported");
    }
    if object
        .get("pattern")
        .and_then(Value::as_str)
        .is_some_and(|pattern| pattern.len() > 256)
    {
        return invalid("schema pattern exceeds limit");
    }
    if object
        .get("examples")
        .is_some_and(|examples| !examples.is_array())
    {
        return invalid("schema examples must be an array");
    }
    // jsonschema 0.18 searches arbitrary annotation objects for $id. Omit
    // arbitrary annotations from its compile-only copy so instance data such
    // as default: {"$id": "not a URL"} cannot become a schema identifier.
    // Original signed schemas remain unchanged in Declarations.
    let mut compiled = object.clone();
    compiled.retain(|key, _| {
        matches!(
            key.as_str(),
            "$schema"
                | "$comment"
                | "title"
                | "description"
                | "readOnly"
                | "writeOnly"
                | "contentEncoding"
                | "contentMediaType"
                | "type"
                | "enum"
                | "const"
                | "multipleOf"
                | "maximum"
                | "exclusiveMaximum"
                | "minimum"
                | "exclusiveMinimum"
                | "maxLength"
                | "minLength"
                | "pattern"
                | "format"
                | "items"
                | "additionalItems"
                | "maxItems"
                | "minItems"
                | "uniqueItems"
                | "contains"
                | "maxProperties"
                | "minProperties"
                | "required"
                | "properties"
                | "patternProperties"
                | "additionalProperties"
                | "dependencies"
                | "propertyNames"
                | "allOf"
                | "anyOf"
                | "oneOf"
                | "not"
                | "if"
                | "then"
                | "else"
                | "definitions"
        )
    });
    // Follow schema positions, never instance data in const/default/enum or
    // names in a properties map. A property called "$ref" is ordinary data.
    for keyword in [
        "additionalItems",
        "additionalProperties",
        "contains",
        "propertyNames",
        "if",
        "then",
        "else",
        "not",
    ] {
        if let Some(schema) = compiled.get_mut(keyword) {
            *schema = schema_for_compilation(schema)?;
        }
    }
    if let Some(items) = compiled.get_mut("items") {
        if let Some(schemas) = items.as_array_mut() {
            for schema in schemas {
                *schema = schema_for_compilation(schema)?;
            }
        } else {
            *items = schema_for_compilation(items)?;
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(schemas) = compiled.get_mut(keyword).and_then(Value::as_array_mut) {
            for schema in schemas {
                *schema = schema_for_compilation(schema)?;
            }
        }
    }
    for keyword in [
        "properties",
        "patternProperties",
        "definitions",
        "dependencies",
    ] {
        if let Some(schemas) = compiled.get_mut(keyword).and_then(Value::as_object_mut) {
            for (name, schema) in schemas {
                if keyword == "patternProperties" && name.len() > 256 {
                    return invalid("schema pattern exceeds limit");
                }
                *schema = schema_for_compilation(schema)?;
            }
        }
    }
    Ok(Value::Object(compiled))
}

fn check_names<'a>(names: impl Iterator<Item = &'a str>, limits: &Limits) -> Result<()> {
    let mut seen = BTreeSet::new();
    for name in names {
        if !valid_function_name(name) || !seen.insert(name) {
            return invalid("declaration names must be unique local function names");
        }
        if seen.len() > limits.max_declarations {
            return Err(PackageError::new(
                ErrorCode::ArchiveLimit,
                "declaration count exceeds limit",
            ));
        }
    }
    Ok(())
}

fn invalid<T>(message: &str) -> Result<T> {
    Err(PackageError::new(ErrorCode::InvalidSchema, message))
}
