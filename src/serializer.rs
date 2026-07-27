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

    fn cache_json<'a>(
        &'a self,
        db: &'a SqliteBackend,
        value: Value,
    ) -> Pin<Box<dyn Future<Output = che_orm::Result<()>> + Send + 'a>>;
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

    fn cache_json<'a>(
        &'a self,
        db: &'a SqliteBackend,
        value: Value,
    ) -> Pin<Box<dyn Future<Output = che_orm::Result<()>> + Send + 'a>> {
        Box::pin(async move { upsert_normalized_model(db, (self.serializer)(), value).await })
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
    pub ts_type: Option<&'static str>,
    pub input_ts_type: Option<&'static str>,
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
            ts_type: None,
            input_ts_type: None,
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
            ts_type: None,
            input_ts_type: None,
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
            ts_type: None,
            input_ts_type: None,
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

    pub const fn ts_type(mut self, ts_type: &'static str) -> Self {
        self.ts_type = Some(ts_type);
        self
    }

    pub const fn input_ts_type(mut self, ts_type: &'static str) -> Self {
        self.input_ts_type = Some(ts_type);
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

    pub fn cache_values(&self, value: Value) -> Result<Vec<(&'static str, SqliteValue)>> {
        cache_values::<M>(value, self.fields)
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

pub async fn upsert_normalized_model<M>(
    db: &SqliteBackend,
    serializer: ModelSerializer<M>,
    value: Value,
) -> che_orm::Result<()>
where
    M: Model + SqliteModel<Id = i64>,
{
    let Some(object) = value.as_object() else {
        return Ok(());
    };

    for field in serializer.fields() {
        if let Some(relation) = field.relation {
            if let Some(nested) = object.get(field.name).and_then(Value::as_object) {
                relation
                    .cache_json(db, Value::Object(nested.clone()))
                    .await?;
            }
        }
    }

    let merged_object = merge_with_existing::<M>(db, serializer, object).await?;
    let values = match serializer.cache_values(Value::Object(merged_object)) {
        Ok(values) => values,
        Err(_) => return Ok(()),
    };
    if values.is_empty() {
        return Ok(());
    }

    upsert_values::<M>(db, values).await
}

async fn merge_with_existing<M>(
    db: &SqliteBackend,
    serializer: ModelSerializer<M>,
    object: &Map<String, Value>,
) -> che_orm::Result<Map<String, Value>>
where
    M: Model + SqliteModel<Id = i64>,
{
    let mut merged = object.clone();
    let Some(id) = merged.get("id").and_then(Value::as_i64) else {
        return Ok(merged);
    };

    if let Ok(existing) = M::objects(db).get(id).await {
        let existing = serializer.to_json_async(db, &existing).await?;
        if let Some(existing_object) = existing.as_object() {
            for (key, value) in existing_object {
                merged.entry(key.clone()).or_insert_with(|| value.clone());
            }
        }
    }

    Ok(merged)
}

fn cache_values<M: Model>(
    value: Value,
    fields: &[Field],
) -> Result<Vec<(&'static str, SqliteValue)>> {
    let object = value.as_object().ok_or(SerializerError::ExpectedObject)?;
    let mut values = Vec::new();

    for field in fields {
        if field.write_only || field.relation.is_some() {
            continue;
        }

        let Some(model_field) = find_model_field(M::fields(), field.source)
            .or_else(|| find_model_field(M::fields(), field.name))
        else {
            continue;
        };

        let raw_value = lookup_remote_value(object, field.source)
            .or_else(|| lookup_remote_value(object, field.name));

        let value = match raw_value {
            Some(Value::Null) | None if field.nullable || model_field.nullable => SqliteValue::Null,
            Some(Value::Null) | None => {
                if let Some(default) = field.default {
                    sqlite_value_from_json(model_field, default())?
                } else {
                    return Err(SerializerError::MissingField(field.name.to_string()));
                }
            }
            Some(value) if field.json => SqliteValue::String(value.to_string()),
            Some(Value::Object(object)) if model_field.ty == FieldType::Integer => {
                sqlite_value_from_object(object).ok_or_else(|| SerializerError::InvalidType {
                    field: field.name.to_string(),
                    expected: "integer",
                })?
            }
            Some(Value::Object(object)) if model_field.ty == FieldType::Text => {
                SqliteValue::String(
                    coerce_text_value(&object)
                        .unwrap_or_else(|| Value::Object(object.clone()).to_string()),
                )
            }
            Some(value) => sqlite_value_from_json(model_field, value.clone())?,
        };

        values.push((model_field.db_name, value));
    }

    Ok(values)
}

async fn upsert_values<M>(
    db: &SqliteBackend,
    values: Vec<(&'static str, SqliteValue)>,
) -> che_orm::Result<()>
where
    M: Model + SqliteModel<Id = i64>,
{
    let pk = M::primary_key().ok_or(che_orm::Error::MissingPrimaryKey)?;
    let columns = values.iter().map(|(name, _)| *name).collect::<Vec<_>>();
    let placeholders = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let updates = columns
        .iter()
        .copied()
        .filter(|name| *name != pk.db_name)
        .map(|name| format!("{name} = excluded.{name}"))
        .collect::<Vec<_>>();
    let conflict_clause = if updates.is_empty() {
        "ON CONFLICT DO NOTHING".to_string()
    } else {
        format!(
            "ON CONFLICT({}) DO UPDATE SET {}",
            pk.db_name,
            updates.join(", ")
        )
    };
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({}) {}",
        M::table_name(),
        columns.join(", "),
        placeholders,
        conflict_clause,
    );
    let query = values.into_iter().fold(
        che_orm::__private::sqlx::query(&sql),
        |query, (_, value)| match value {
            SqliteValue::I64(value) => query.bind(value),
            SqliteValue::String(value) => query.bind(value),
            SqliteValue::Bool(value) => query.bind(value),
            SqliteValue::F64(value) => query.bind(value),
            SqliteValue::Null => query.bind(Option::<i64>::None),
        },
    );
    query.execute(db.pool()).await?;
    Ok(())
}

fn sqlite_value_from_json(field: &FieldInfo, value: Value) -> Result<SqliteValue> {
    if value.is_null() {
        return Ok(SqliteValue::Null);
    }

    match field.ty {
        FieldType::Integer => value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
            .map(SqliteValue::from)
            .or_else(|| match value {
                Value::Object(object) => sqlite_value_from_object(&object),
                _ => None,
            })
            .ok_or_else(|| SerializerError::InvalidType {
                field: field.rust_name.to_string(),
                expected: "integer",
            }),
        FieldType::Text => match value {
            Value::String(text) => Ok(SqliteValue::from(text)),
            Value::Object(object) => Ok(SqliteValue::String(
                coerce_text_value(&object)
                    .unwrap_or_else(|| Value::Object(object.clone()).to_string()),
            )),
            _ => Err(SerializerError::InvalidType {
                field: field.rust_name.to_string(),
                expected: "string",
            }),
        },
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

fn coerce_text_value(object: &Map<String, Value>) -> Option<String> {
    for key in ["name", "short_name", "title", "label", "value"] {
        if let Some(text) = object.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }

    object
        .get("id")
        .and_then(|value| value.as_i64().map(|value| value.to_string()))
        .or_else(|| {
            object
                .get("id")
                .and_then(|value| value.as_u64().map(|value| value.to_string()))
        })
}

fn sqlite_value_from_object(object: &Map<String, Value>) -> Option<SqliteValue> {
    object
        .get("id")
        .and_then(|value| value.as_i64().map(SqliteValue::from))
        .or_else(|| {
            object
                .get("id")
                .and_then(|value| value.as_u64().and_then(|value| i64::try_from(value).ok()))
                .map(SqliteValue::from)
        })
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
            raw_value
                .cloned()
                .unwrap_or_else(|| default_remote_value::<M>(field))
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
        Some(Value::Object(object)) => coerce_text_value(object)
            .map(Value::String)
            .unwrap_or_else(|| Value::Object(object.clone())),
        Some(value) if !value.is_null() => value.clone(),
        _ => default_remote_value::<M>(field),
    }
}

fn default_remote_value<M: Model>(field: &Field) -> Value {
    if let Some(default) = field.default {
        return default();
    }

    let model_field = M::fields().iter().find(|model_field| {
        model_field.db_name == field.name || model_field.rust_name == field.name
    });

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
