//! Typed application operations over one framework-owned invoke command.
//! Register once for runtime dispatch and TypeScript generation. UUIDs and custom
//! DTOs are supported independently of the ordinary i64 resource CRUD API.
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use schemars::{
    JsonSchema,
    generate::{Contract, SchemaSettings},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::{ApiError, ApiResult};

type ResponseFuture = Pin<Box<dyn Future<Output = ApiResult<Value>> + Send>>;
type Handler<S> = Arc<dyn Fn(Arc<S>, Value) -> ResponseFuture + Send + Sync>;

#[derive(Clone)]
pub struct CommandMetadata {
    pub name: String,
    pub input: Value,
    pub output: Value,
}

pub struct CommandRouter<S> {
    handlers: BTreeMap<String, Handler<S>>,
    metadata: Vec<CommandMetadata>,
}

impl<S> Default for CommandRouter<S> {
    fn default() -> Self {
        Self {
            handlers: BTreeMap::new(),
            metadata: Vec::new(),
        }
    }
}

impl<S: Send + Sync + 'static> CommandRouter<S> {
    pub fn new() -> Self {
        Self::default()
    }

    /// State is supplied only at runtime. Management can inspect this same
    /// registry without opening databases, initializing Tauri or authenticating.
    pub fn command<I, O, F, Fut>(mut self, name: &str, handler: F) -> ApiResult<Self>
    where
        I: DeserializeOwned + JsonSchema + Send + 'static,
        O: Serialize + JsonSchema + 'static,
        F: Fn(Arc<S>, I) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = ApiResult<O>> + Send + 'static,
    {
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            return Err(ApiError::bad_request("invalid invoke operation name"));
        }
        if self.handlers.contains_key(name) {
            return Err(ApiError::bad_request(format!(
                "duplicate invoke operation {name}"
            )));
        }
        let schema = |contract| {
            let mut settings = SchemaSettings::default();
            settings.contract = contract;
            settings
        };
        self.metadata.push(CommandMetadata {
            name: name.into(),
            input: schema(Contract::Deserialize)
                .into_generator()
                .into_root_schema_for::<I>()
                .to_value(),
            output: schema(Contract::Serialize)
                .into_generator()
                .into_root_schema_for::<O>()
                .to_value(),
        });
        let handler = Arc::new(handler);
        self.handlers.insert(
            name.into(),
            Arc::new(move |state, payload| {
                let handler = handler.clone();
                Box::pin(async move {
                    let input = serde_json::from_value(payload)
                        .map_err(|error| ApiError::bad_request(error.to_string()))?;
                    let output = handler(state, input).await?;
                    serde_json::to_value(output)
                        .map_err(|error| ApiError::internal(error.to_string()))
                })
            }),
        );
        Ok(self)
    }

    pub fn metadata(&self) -> &[CommandMetadata] {
        &self.metadata
    }

    pub async fn dispatch(&self, state: Arc<S>, request: CommandRequest) -> ApiResult<Value> {
        let handler = self
            .handlers
            .get(&request.operation)
            .ok_or_else(|| ApiError::not_found("unknown invoke operation"))?;
        handler(state, request.payload).await
    }

    pub fn bind(self, state: Arc<S>) -> TauriCommands {
        TauriCommands {
            dispatch: Arc::new(move |request| {
                let state = state.clone();
                let handler = self.handlers.get(&request.operation).cloned();
                Box::pin(async move {
                    let handler =
                        handler.ok_or_else(|| ApiError::not_found("unknown invoke operation"))?;
                    handler(state, request.payload).await
                })
            }),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct CommandRequest {
    pub operation: String,
    #[serde(default)]
    pub payload: Value,
}

pub struct TauriCommands {
    dispatch: Arc<dyn Fn(CommandRequest) -> ResponseFuture + Send + Sync>,
}

impl TauriCommands {
    pub async fn dispatch(&self, request: CommandRequest) -> ApiResult<Value> {
        (self.dispatch)(request).await
    }
}

#[tauri::command]
pub async fn che_invoke(
    commands: tauri::State<'_, TauriCommands>,
    request: CommandRequest,
) -> ApiResult<Value> {
    commands.dispatch(request).await
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::internal(message)
}
fn identifier(name: &str) -> bool {
    if [
        "null",
        "string",
        "number",
        "boolean",
        "unknown",
        "never",
        "any",
        "void",
        "undefined",
        "object",
        "default",
        "class",
        "export",
        "import",
        "type",
        "interface",
        "const",
        "let",
        "function",
        "new",
        "return",
        "extends",
        "enum",
    ]
    .contains(&name)
    {
        return false;
    }
    let mut bytes = name.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || b"_$".contains(&byte))
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || b"_$".contains(&byte))
}
fn normalized(mut schema: Value, root: &str) -> Value {
    fn references(value: &mut Value, root: &str) {
        if let Value::Object(object) = value {
            if object.get("$ref").and_then(Value::as_str) == Some("#") {
                object.insert("$ref".into(), Value::String(format!("#/$defs/{root}")));
            }
            // Only traverse schema locations, never literal JSON inside
            // const/default/enum/examples (which may itself contain $ref).
            for (keyword, value) in object.iter_mut() {
                match keyword.as_str() {
                    "$defs" | "definitions" | "properties" | "patternProperties"
                    | "dependentSchemas" => {
                        if let Value::Object(schemas) = value {
                            for schema in schemas.values_mut() {
                                references(schema, root);
                            }
                        }
                    }
                    "allOf" | "anyOf" | "oneOf" | "prefixItems" => {
                        if let Value::Array(schemas) = value {
                            for schema in schemas {
                                references(schema, root);
                            }
                        }
                    }
                    "items"
                    | "additionalProperties"
                    | "unevaluatedProperties"
                    | "contains"
                    | "propertyNames"
                    | "not"
                    | "if"
                    | "then"
                    | "else" => references(value, root),
                    _ => {}
                }
            }
        }
    }
    references(&mut schema, root);
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
        object.remove("$defs");
    }
    schema
}

/// JSON Schema metadata comes from schemars' deserialize/serialize contracts,
/// including Serde field names, enum tagging and optional/nullable fields.
pub fn typescript(metadata: &[CommandMetadata]) -> ApiResult<String> {
    let mut definitions = BTreeMap::<String, Value>::new();
    let mut register = |name: &str, schema: &Value, root: &str| -> ApiResult<()> {
        if !identifier(name) {
            return Err(invalid(format!("invalid TypeScript schema name {name}")));
        }
        let schema = normalized(schema.clone(), root);
        if let Some(previous) = definitions.insert(name.into(), schema.clone())
            && previous != schema
        {
            return Err(invalid(format!(
                "conflicting schema definitions for {name}; give input and output DTOs distinct schemars names"
            )));
        }
        Ok(())
    };
    for command in metadata {
        for schema in [&command.input, &command.output] {
            let root = schema["title"].as_str().unwrap_or("never");
            if let Some(definitions) = schema["$defs"].as_object() {
                for (name, schema) in definitions {
                    register(name, schema, root)?;
                }
            }
            if let Some(name) = schema["title"].as_str().filter(|name| identifier(name)) {
                register(name, schema, root)?;
            }
        }
    }
    // Generated helpers share this module with DTOs. Allocate stable aliases
    // for colliding names and reserve existing DTO names before picking aliases.
    let reserved = [
        "InvokeError",
        "Promise",
        "invoke",
        "errorMessage",
        "commands",
    ];
    let mut symbols = BTreeMap::new();
    let mut occupied: std::collections::BTreeSet<String> = definitions
        .keys()
        .cloned()
        .chain(reserved.iter().map(|name| (*name).to_owned()))
        .collect();
    for name in definitions.keys() {
        let mut symbol = name.clone();
        if reserved.contains(&name.as_str()) {
            symbol = format!("ApiDto{name}");
            let base = symbol.clone();
            let mut suffix = 2;
            while occupied.contains(&symbol) {
                symbol = format!("{base}{suffix}");
                suffix += 1;
            }
            occupied.insert(symbol.clone());
        }
        symbols.insert(name.clone(), symbol);
    }
    let mut out = String::from(
        "// Generated by che-tauri. Do not edit.\nimport { invoke } from '@tauri-apps/api/core';\n\n",
    );
    for (name, schema) in &definitions {
        out.push_str(&format!(
            "export type {} = {};\n\n",
            symbols[name],
            ts_type(schema, name, &symbols)?
        ));
    }
    out.push_str(include_str!("invoke_errors.ts"));
    out.push_str("\nexport const commands = {\n");
    for command in metadata {
        let input = root_type(&command.input, &symbols)?;
        let output = root_type(&command.output, &symbols)?;
        let default = if input == "null" { " = null" } else { "" };
        let name = serde_json::to_string(&command.name).unwrap();
        out.push_str(&format!("  {name}: (payload: {input}{default}): Promise<{output}> => invoke('che_invoke', {{ request: {{ operation: {name}, payload }} }}),\n"));
    }
    out.push_str("};\n");
    Ok(out)
}

fn root_type(schema: &Value, symbols: &BTreeMap<String, String>) -> ApiResult<String> {
    match schema["title"].as_str() {
        Some(name) if identifier(name) => Ok(symbols[name].clone()),
        _ => ts_type(schema, "never", symbols),
    }
}

fn ts_type(schema: &Value, root: &str, symbols: &BTreeMap<String, String>) -> ApiResult<String> {
    if schema == &Value::Bool(true) {
        return Ok("unknown".into());
    }
    if schema == &Value::Bool(false) {
        return Ok("never".into());
    }
    for (keyword, operator) in [("anyOf", " | "), ("oneOf", " | "), ("allOf", " & ")] {
        if let Some(variants) = schema[keyword].as_array() {
            let variants = variants
                .iter()
                .map(|variant| ts_type(variant, root, symbols))
                .collect::<ApiResult<Vec<_>>>()?;
            let combined = format!("({})", variants.join(operator));
            // Composition keywords constrain the same value as their siblings.
            // Serde flatten, for example, emits common properties beside oneOf.
            let mut siblings = schema.clone();
            siblings.as_object_mut().unwrap().remove(keyword);
            if [
                "$ref",
                "const",
                "enum",
                "type",
                "properties",
                "additionalProperties",
                "anyOf",
                "oneOf",
                "allOf",
            ]
            .iter()
            .any(|key| siblings.get(key).is_some())
            {
                return Ok(format!(
                    "({} & {combined})",
                    ts_type(&siblings, root, symbols)?
                ));
            }
            return Ok(combined);
        }
    }
    if let Some(reference) = schema["$ref"].as_str() {
        if reference == "#" {
            return Ok(symbols.get(root).cloned().unwrap_or_else(|| root.into()));
        }
        let name = reference
            .strip_prefix("#/$defs/")
            .ok_or_else(|| invalid(format!("unsupported schema reference {reference}")))?;
        if !identifier(name) {
            return Err(invalid("invalid schema reference"));
        }
        return symbols
            .get(name)
            .cloned()
            .ok_or_else(|| invalid(format!("unknown schema reference {reference}")));
    }
    if let Some(value) = schema.get("const") {
        return Ok(value.to_string());
    }
    if let Some(values) = schema["enum"].as_array() {
        return Ok(if values.is_empty() {
            "never".into()
        } else {
            values
                .iter()
                .map(Value::to_string)
                .collect::<Vec<_>>()
                .join(" | ")
        });
    }
    if let Some(types) = schema["type"].as_array() {
        let variants = types
            .iter()
            .map(|kind| {
                let mut variant = schema.clone();
                variant["type"] = kind.clone();
                ts_type(&variant, root, symbols)
            })
            .collect::<ApiResult<Vec<_>>>()?;
        return Ok(format!("({})", variants.join(" | ")));
    }
    Ok(match schema["type"].as_str() {
        Some("null") => "null".into(),
        Some("boolean") => "boolean".into(),
        Some("integer" | "number") => "number".into(),
        Some("string") => "string".into(),
        Some("array") => {
            if let Some(items) = schema["prefixItems"].as_array() {
                format!(
                    "[{}]",
                    items
                        .iter()
                        .map(|item| ts_type(item, root, symbols))
                        .collect::<ApiResult<Vec<_>>>()?
                        .join(", ")
                )
            } else {
                format!(
                    "({})[]",
                    ts_type(
                        schema.get("items").unwrap_or(&Value::Bool(true)),
                        root,
                        symbols
                    )?
                )
            }
        }
        Some("object") | None
            if schema.get("properties").is_some() || schema["type"] == "object" =>
        {
            let required = schema["required"].as_array();
            let mut fields = Vec::new();
            if let Some(properties) = schema["properties"].as_object() {
                for (name, field) in properties {
                    let optional = if required
                        .is_some_and(|required| required.iter().any(|item| item == name))
                    {
                        ""
                    } else {
                        "?"
                    };
                    fields.push(format!(
                        "{}{}: {}",
                        serde_json::to_string(name).unwrap(),
                        optional,
                        ts_type(field, root, symbols)?
                    ));
                }
            }
            let object = format!("{{ {} }}", fields.join("; "));
            match schema.get("additionalProperties") {
                Some(additional) if additional.is_object() || additional == &Value::Bool(true) => {
                    // Index signatures and array syntax cannot be shadowed by
                    // application DTOs named Record or Array.
                    let record = format!(
                        "{{ [key: string]: {} }}",
                        ts_type(additional, root, symbols)?
                    );
                    if fields.is_empty() {
                        record
                    } else {
                        format!("({object} & {record})")
                    }
                }
                _ => object,
            }
        }
        None if schema.as_object().is_some_and(|object| {
            object
                .keys()
                .all(|key| ["description", "default", "title", "$schema"].contains(&key.as_str()))
        }) =>
        {
            "unknown".into()
        }
        _ => return Err(invalid(format!("unsupported TypeScript schema: {schema}"))),
    })
}
