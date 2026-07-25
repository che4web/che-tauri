use std::{future::Future, marker::PhantomData, pin::Pin};

use che_orm::{FieldInfo, FieldType, Model, SqliteBackend, SqliteModel, SqliteValue};
use serde_json::{Map, Value};

pub trait RelatedSerializer: std::fmt::Debug + Send + Sync {
    fn model_name(&self) -> &'static str;

    fn serialize<'a>(
        &'a self,
        db: &'a SqliteBackend,
        id: i64,
    ) -> Pin<Box<dyn Future<Output = che_orm::Result<Value>> + Send + 'a>>;
}

#[derive(Clone, Copy)]
pub struct RelatedModel<M> {
    serializer: fn() -> ModelSerializer<M>,
    _model: PhantomData<M>,
}

impl<M> std::fmt::Debug for RelatedModel<M> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelatedModel")
            .field("model", &std::any::type_name::<M>())
            .finish_non_exhaustive()
    }
}

impl<M> RelatedModel<M> {
    pub const fn new(serializer: fn() -> ModelSerializer<M>) -> Self {
        Self {
            serializer,
            _model: PhantomData,
        }
    }
}

impl<M> RelatedSerializer for RelatedModel<M>
where
    M: SqliteModel<Id = i64>,
{
    fn model_name(&self) -> &'static str {
        std::any::type_name::<M>()
            .rsplit("::")
            .next()
            .unwrap_or("Model")
    }

    fn serialize<'a>(
        &'a self,
        db: &'a SqliteBackend,
        id: i64,
    ) -> Pin<Box<dyn Future<Output = che_orm::Result<Value>> + Send + 'a>> {
        Box::pin(async move {
            let model = M::objects(db).get(id).await?;
            (self.serializer)().to_json_async(db, &model).await
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub source: &'static str,
    pub required: bool,
    pub read_only: bool,
    pub write_only: bool,
    pub nullable: bool,
    pub max_length: Option<u32>,
    pub relation: Option<&'static dyn RelatedSerializer>,
    pub json: bool,
    default: Option<fn() -> Value>,
}

impl Field {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            source: name,
            required: true,
            read_only: false,
            write_only: false,
            nullable: false,
            max_length: None,
            relation: None,
            json: false,
            default: None,
        }
    }

    pub const fn json(name: &'static str) -> Self {
        Self {
            name,
            source: name,
            required: true,
            read_only: false,
            write_only: false,
            nullable: false,
            max_length: None,
            relation: None,
            json: true,
            default: None,
        }
    }

    pub const fn related(
        name: &'static str,
        source: &'static str,
        relation: &'static dyn RelatedSerializer,
    ) -> Self {
        Self {
            name,
            source,
            required: false,
            read_only: true,
            write_only: false,
            nullable: false,
            max_length: None,
            relation: Some(relation),
            json: false,
            default: None,
        }
    }

    pub const fn source(mut self, source: &'static str) -> Self {
        self.source = source;
        self
    }

    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub const fn read_only(mut self) -> Self {
        self.read_only = true;
        self.required = false;
        self
    }

    pub const fn write_only(mut self) -> Self {
        self.write_only = true;
        self
    }

    pub const fn nullable(mut self) -> Self {
        self.nullable = true;
        self
    }

    pub const fn max_length(mut self, max_length: u32) -> Self {
        self.max_length = Some(max_length);
        self
    }

    pub const fn default(mut self, default: fn() -> Value) -> Self {
        self.default = Some(default);
        self
    }

    pub const fn without_default(mut self) -> Self {
        self.default = None;
        self
    }

    pub const fn has_default(&self) -> bool {
        self.default.is_some()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SerializerError {
    #[error("expected a JSON object")]
    ExpectedObject,

    #[error("unknown field: {0}")]
    UnknownField(String),

    #[error("field is read-only: {0}")]
    ReadonlyField(String),

    #[error("missing field: {0}")]
    MissingField(String),

    #[error("null is not allowed for field: {0}")]
    NullNotAllowed(String),

    #[error("invalid type for field {field}, expected {expected}")]
    InvalidType {
        field: String,
        expected: &'static str,
    },

    #[error("field {field} exceeds max length {max_length}")]
    MaxLengthExceeded { field: String, max_length: u32 },

    #[error("invalid model field: {0}")]
    InvalidModelField(String),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, SerializerError>;

#[derive(Debug)]
pub struct ModelSerializer<M> {
    fields: &'static [Field],
    _model: PhantomData<M>,
}

impl<M> Clone for ModelSerializer<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for ModelSerializer<M> {}

impl<M: Model> ModelSerializer<M> {
    pub const fn new(fields: &'static [Field]) -> Self {
        Self {
            fields,
            _model: PhantomData,
        }
    }

    pub fn fields(&self) -> &'static [Field] {
        self.fields
    }

    pub fn to_json(&self, model: &M) -> Value {
        serialize_model(model, self.fields)
    }

    pub async fn to_json_async(&self, db: &SqliteBackend, model: &M) -> che_orm::Result<Value> {
        serialize_model_async(db, model, self.fields).await
    }

    pub fn validate_json(&self, value: Value) -> Result<Map<String, Value>> {
        validate_object::<M>(value, self.fields)
    }

    pub fn create_values(&self, value: Value) -> Result<Vec<(&'static str, SqliteValue)>> {
        validated_values::<M>(value, self.fields)
    }

    pub fn update_values(&self, value: Value) -> Result<Vec<(&'static str, SqliteValue)>> {
        let fields = self
            .fields
            .iter()
            .map(|field| {
                if field.read_only {
                    *field
                } else {
                    field.required(false).without_default()
                }
            })
            .collect::<Vec<_>>();

        validated_values::<M>(value, &fields)
    }

    pub fn normalize_remote_value(&self, value: Value) -> Value {
        normalize_remote_object::<M>(value, self.fields)
    }
}

pub fn normalize_remote_object<M: Model>(value: Value, fields: &[Field]) -> Value {
    let Some(remote) = value.as_object() else {
        return value;
    };

    let mut object = Map::new();

    for field in fields {
        if field.write_only {
            continue;
        }

        let raw_value = lookup_remote_value(remote, field.source)
            .or_else(|| lookup_remote_value(remote, field.name));
        let value = if field.json {
            raw_value.cloned().unwrap_or_else(|| default_remote_value::<M>(field))
        } else if field.relation.is_some() {
            match raw_value {
                Some(Value::Object(_)) => raw_value.cloned().unwrap_or(Value::Null),
                _ => Value::Null,
            }
        } else {
            normalize_remote_field_value::<M>(field, raw_value)
        };

        object.insert(field.name.to_string(), value);
    }

    Value::Object(object)
}

fn lookup_remote_value<'a>(object: &'a Map<String, Value>, path: &str) -> Option<&'a Value> {
    let mut parts = path.split('.');
    let first = parts.next()?;
    let mut value = object.get(first)?;

    for part in parts {
        value = value.as_object()?.get(part)?;
    }

    Some(value)
}

fn normalize_remote_field_value<M: Model>(field: &Field, value: Option<&Value>) -> Value {
    match value {
        Some(Value::Object(object)) if field.name.ends_with("_id") => {
            object.get("id").cloned().unwrap_or(Value::Null)
        }
        Some(value) if !value.is_null() => value.clone(),
        _ => default_remote_value::<M>(field),
    }
}

fn default_remote_value<M: Model>(field: &Field) -> Value {
    if let Some(default) = field.default {
        return default();
    }

    let model_field = M::fields()
        .iter()
        .find(|model_field| model_field.db_name == field.name || model_field.rust_name == field.name);

    match model_field {
        Some(model_field) if model_field.nullable || field.nullable => Value::Null,
        Some(model_field) => match model_field.ty {
            FieldType::Text => Value::String(String::new()),
            FieldType::Integer => Value::from(0),
            FieldType::Boolean => Value::from(false),
            FieldType::Real => Value::from(0.0),
        },
        None if field.nullable => Value::Null,
        None => Value::Null,
    }
}

pub fn serialize_model<M: Model>(model: &M, fields: &[Field]) -> Value {
    let mut object = Map::new();

    for field in fields {
        if field.write_only || field.relation.is_some() {
            continue;
        }

        let value = model
            .get_value(field.source)
            .or_else(|| model.get_value(field.name))
            .unwrap_or(Value::Null);
        object.insert(field.name.to_string(), serialize_json_field(field, value));
    }

    Value::Object(object)
}

pub async fn serialize_model_async<M: Model>(
    db: &SqliteBackend,
    model: &M,
    fields: &[Field],
) -> che_orm::Result<Value> {
    let mut object = Map::new();

    for field in fields {
        if field.write_only {
            continue;
        }

        if let Some(relation) = field.relation {
            let value = match model
                .get_value(field.source)
                .or_else(|| model.get_value(field.name))
                .and_then(|value| value.as_i64())
            {
                Some(id) => relation.serialize(db, id).await?,
                None => Value::Null,
            };
            object.insert(field.name.to_string(), value);
            continue;
        }

        let value = model
            .get_value(field.source)
            .or_else(|| model.get_value(field.name))
            .unwrap_or(Value::Null);
        object.insert(field.name.to_string(), serialize_json_field(field, value));
    }

    Ok(Value::Object(object))
}

fn serialize_json_field(field: &Field, value: Value) -> Value {
    if !field.json {
        return value;
    }

    match value {
        Value::String(string) => serde_json::from_str(&string).unwrap_or(Value::Null),
        Value::Null => Value::Null,
        value => value,
    }
}

pub fn validate_object<M: Model>(value: Value, fields: &[Field]) -> Result<Map<String, Value>> {
    let object = value.as_object().ok_or(SerializerError::ExpectedObject)?;

    for key in object.keys() {
        if !fields.iter().any(|field| field.name == key.as_str()) {
            return Err(SerializerError::UnknownField(key.clone()));
        }
    }

    let model_fields = M::fields();
    let mut validated = Map::new();

    for field in fields {
        let model_field = find_model_field(model_fields, field.source)
            .ok_or_else(|| SerializerError::InvalidModelField(field.source.to_string()))?;

        if field.read_only {
            if object.contains_key(field.name) {
                return Err(SerializerError::ReadonlyField(field.name.to_string()));
            }
            continue;
        }

        match object.get(field.name) {
            Some(Value::Null) => {
                if !field.nullable {
                    return Err(SerializerError::NullNotAllowed(field.name.to_string()));
                }
                validated.insert(field.source.to_string(), Value::Null);
            }
            Some(value) => {
                if field.json {
                    validated.insert(field.source.to_string(), Value::String(value.to_string()));
                    continue;
                }

                validate_type(field.name, model_field.ty, value)?;
                if let Some(max_length) = field.max_length {
                    validate_max_length(field.name, max_length, value)?;
                }
                validated.insert(field.source.to_string(), value.clone());
            }
            None => {
                if let Some(default) = field.default {
                    validated.insert(field.source.to_string(), default());
                } else if field.required {
                    return Err(SerializerError::MissingField(field.name.to_string()));
                }
            }
        }
    }

    Ok(validated)
}

fn validated_values<M: Model>(
    value: Value,
    fields: &[Field],
) -> Result<Vec<(&'static str, SqliteValue)>> {
    let data = validate_object::<M>(value, fields)?;
    let mut values = Vec::new();

    for (name, value) in data {
        let field = find_model_field(M::fields(), &name)
            .ok_or_else(|| SerializerError::InvalidModelField(name.clone()))?;
        values.push((field.db_name, json_to_sqlite_value(field, value)?));
    }

    Ok(values)
}

fn json_to_sqlite_value(field: &FieldInfo, value: Value) -> Result<SqliteValue> {
    if value.is_null() {
        return Ok(SqliteValue::Null);
    }

    match field.ty {
        FieldType::Integer => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .map(SqliteValue::from)
            .ok_or_else(|| SerializerError::InvalidType {
                field: field.rust_name.to_string(),
                expected: "integer",
            }),
        FieldType::Text => {
            value
                .as_str()
                .map(SqliteValue::from)
                .ok_or_else(|| SerializerError::InvalidType {
                    field: field.rust_name.to_string(),
                    expected: "string",
                })
        }
        FieldType::Boolean => {
            value
                .as_bool()
                .map(SqliteValue::from)
                .ok_or_else(|| SerializerError::InvalidType {
                    field: field.rust_name.to_string(),
                    expected: "boolean",
                })
        }
        FieldType::Real => {
            value
                .as_f64()
                .map(SqliteValue::from)
                .ok_or_else(|| SerializerError::InvalidType {
                    field: field.rust_name.to_string(),
                    expected: "number",
                })
        }
    }
}

fn find_model_field<'a>(fields: &'a [FieldInfo], name: &str) -> Option<&'a FieldInfo> {
    fields
        .iter()
        .find(|field| field.rust_name == name || field.db_name == name)
}

fn validate_type(field: &str, ty: FieldType, value: &Value) -> Result<()> {
    let expected = match ty {
        FieldType::Integer => {
            if value.as_i64().is_some() || value.as_u64().is_some() {
                return Ok(());
            }
            "integer"
        }
        FieldType::Text => {
            if value.as_str().is_some() {
                return Ok(());
            }
            "string"
        }
        FieldType::Boolean => {
            if value.as_bool().is_some() {
                return Ok(());
            }
            "boolean"
        }
        FieldType::Real => {
            if value.as_f64().is_some() {
                return Ok(());
            }
            "number"
        }
    };

    Err(SerializerError::InvalidType {
        field: field.to_string(),
        expected,
    })
}

fn validate_max_length(field: &str, max_length: u32, value: &Value) -> Result<()> {
    let Some(text) = value.as_str() else {
        return Ok(());
    };

    if text.chars().count() > max_length as usize {
        return Err(SerializerError::MaxLengthExceeded {
            field: field.to_string(),
            max_length,
        });
    }

    Ok(())
}
