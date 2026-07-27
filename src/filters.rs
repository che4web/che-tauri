use std::{collections::HashMap, marker::PhantomData};

use che_orm::{FieldInfo, FieldType, Model, QueryBuilder, SqliteModel, SqliteValue};
use serde_json::Value;

use crate::error::ApiResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lookup {
    Exact,
    Contains,
    Gt,
    Gte,
    Lt,
    Lte,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteFilterMode {
    Forward,
    LocalOnly,
    RemoteOnly,
    Disabled,
}

#[derive(Debug, Clone, Copy)]
pub struct Filter {
    pub name: &'static str,
    pub source: &'static str,
    pub lookup: Lookup,
    pub remote_name: Option<&'static str>,
    pub remote_mode: RemoteFilterMode,
}

impl Filter {
    pub const fn exact(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Exact)
    }

    pub const fn contains(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Contains)
    }

    pub const fn gt(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Gt)
    }

    pub const fn gte(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Gte)
    }

    pub const fn lt(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Lt)
    }

    pub const fn lte(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Lte)
    }

    pub const fn remote_only(name: &'static str) -> Self {
        Self::new(name, name, Lookup::Exact).remote_only_mode()
    }

    pub const fn exact_source(name: &'static str, source: &'static str) -> Self {
        Self::new(name, source, Lookup::Exact)
    }

    pub const fn new(name: &'static str, source: &'static str, lookup: Lookup) -> Self {
        Self {
            name,
            source,
            lookup,
            remote_name: None,
            remote_mode: RemoteFilterMode::Forward,
        }
    }

    pub const fn remote(mut self, name: &'static str) -> Self {
        self.remote_name = Some(name);
        self
    }

    pub const fn local_only(mut self) -> Self {
        self.remote_mode = RemoteFilterMode::LocalOnly;
        self
    }

    pub const fn remote_disabled(mut self) -> Self {
        self.remote_mode = RemoteFilterMode::Disabled;
        self
    }

    const fn remote_only_mode(mut self) -> Self {
        self.remote_mode = RemoteFilterMode::RemoteOnly;
        self
    }

    pub fn query_name(&self) -> String {
        match self.lookup {
            Lookup::Exact => self.name.to_string(),
            Lookup::Contains => format!("{}__contains", self.name),
            Lookup::Gt => format!("{}__gt", self.name),
            Lookup::Gte => format!("{}__gte", self.name),
            Lookup::Lt => format!("{}__lt", self.name),
            Lookup::Lte => format!("{}__lte", self.name),
        }
    }
}

#[derive(Debug)]
pub struct FilterSet<M> {
    filters: &'static [Filter],
    remote_ordering: Option<&'static str>,
    _model: PhantomData<M>,
}

impl<M> Clone for FilterSet<M> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<M> Copy for FilterSet<M> {}

impl<M> FilterSet<M>
where
    M: SqliteModel,
{
    pub const fn new(filters: &'static [Filter]) -> Self {
        Self {
            filters,
            remote_ordering: None,
            _model: PhantomData,
        }
    }

    pub const fn remote_ordering(mut self, name: &'static str) -> Self {
        self.remote_ordering = Some(name);
        self
    }

    pub fn filters(&self) -> &'static [Filter] {
        self.filters
    }

    pub fn remote_params(&self, params: &HashMap<String, String>) -> HashMap<String, String> {
        let mut remote = HashMap::new();

        for (name, value) in params {
            if value.is_empty() {
                continue;
            }

            match name.as_str() {
                "limit" | "offset" => {
                    remote.insert(name.clone(), value.clone());
                }
                "ordering" => {
                    remote.insert(
                        self.remote_ordering.unwrap_or("ordering").to_string(),
                        self.remote_ordering_value(value),
                    );
                }
                name => {
                    let Some(filter) = self.filter_by_query_name(name) else {
                        remote.insert(name.to_string(), value.clone());
                        continue;
                    };

                    match filter.remote_mode {
                        RemoteFilterMode::Forward | RemoteFilterMode::RemoteOnly => {
                            remote.insert(filter.remote_query_name(), value.clone());
                        }
                        RemoteFilterMode::LocalOnly | RemoteFilterMode::Disabled => {}
                    }
                }
            }
        }

        remote
    }

    pub fn apply_json(&self, value: Value, params: &HashMap<String, String>) -> Value {
        let local_filters = params
            .iter()
            .filter_map(|(name, value)| {
                let filter = self.filter_by_query_name(name)?;
                if filter.remote_mode == RemoteFilterMode::LocalOnly && !value.is_empty() {
                    Some((filter, value.as_str()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        if local_filters.is_empty() {
            return value;
        }

        let Some(mut object) = value.as_object().cloned() else {
            return value;
        };
        let Some(results) = object.get("results").and_then(Value::as_array) else {
            return Value::Object(object);
        };

        let filtered = results
            .iter()
            .filter(|item| {
                local_filters
                    .iter()
                    .all(|(filter, value)| json_item_matches_filter(item, filter, value))
            })
            .cloned()
            .collect::<Vec<_>>();

        object.insert("count".to_string(), Value::from(filtered.len()));
        object.insert("results".to_string(), Value::Array(filtered));

        Value::Object(object)
    }

    pub fn apply<'db>(
        &self,
        mut query: QueryBuilder<'db, M>,
        params: &HashMap<String, String>,
    ) -> ApiResult<QueryBuilder<'db, M>> {
        for (name, value) in params {
            match name.as_str() {
                "ordering" => {
                    query = self.apply_ordering(query, value)?;
                }
                "limit" => {
                    query = query.limit(parse_u32("limit", value)?);
                }
                "offset" => {
                    query = query.offset(parse_u32("offset", value)?);
                }
                name => {
                    let filter = self
                        .filter_by_query_name(name)
                        .ok_or_else(|| FilterError::UnknownFilter(name.to_string()))?;

                    if filter.remote_mode == RemoteFilterMode::RemoteOnly {
                        continue;
                    }

                    let field = model_field::<M>(filter.source)?;
                    validate_lookup(field, filter.lookup)?;
                    let value = parse_value(field, value)?;
                    query = match filter.lookup {
                        Lookup::Exact => query.eq(filter.source, value),
                        Lookup::Contains => query.contains(filter.source, value),
                        Lookup::Gt => query.gt(filter.source, value),
                        Lookup::Gte => query.gte(filter.source, value),
                        Lookup::Lt => query.lt(filter.source, value),
                        Lookup::Lte => query.lte(filter.source, value),
                    };
                }
            }
        }

        Ok(query)
    }

    fn filter_by_query_name(&self, name: &str) -> Option<&'static Filter> {
        self.filters
            .iter()
            .find(|filter| filter.query_name() == name)
    }

    fn remote_ordering_value(&self, value: &str) -> String {
        let descending = value.starts_with('-');
        let field = value.strip_prefix('-').unwrap_or(value);
        let remote_field = self
            .filters
            .iter()
            .find(|filter| filter.name == field)
            .and_then(|filter| filter.remote_name)
            .unwrap_or(field);

        if descending {
            format!("-{remote_field}")
        } else {
            remote_field.to_string()
        }
    }

    fn apply_ordering<'db>(
        &self,
        query: QueryBuilder<'db, M>,
        value: &str,
    ) -> ApiResult<QueryBuilder<'db, M>> {
        let field = value.strip_prefix('-').unwrap_or(value);
        if !self.filters.iter().any(|filter| filter.name == field) {
            return Err(FilterError::UnknownOrdering(field.to_string()).into());
        }
        let source = self
            .filters
            .iter()
            .find(|filter| filter.name == field)
            .map(|filter| filter.source)
            .unwrap_or(field);
        model_field::<M>(source)?;

        Ok(if value.starts_with('-') {
            query.order_by(&format!("-{source}"))
        } else {
            query.order_by(source)
        })
    }
}

impl Filter {
    fn remote_query_name(&self) -> String {
        self.remote_name
            .map(str::to_string)
            .unwrap_or_else(|| self.query_name())
    }
}

fn json_item_matches_filter(item: &Value, filter: &Filter, expected: &str) -> bool {
    let Some(value) = json_lookup(item, filter.name).or_else(|| json_lookup(item, filter.source))
    else {
        return false;
    };

    match filter.lookup {
        Lookup::Exact => json_value_text(value).is_some_and(|value| value == expected),
        Lookup::Contains => {
            let expected = expected.to_lowercase();
            json_value_text(value).is_some_and(|value| value.to_lowercase().contains(&expected))
        }
        Lookup::Gt | Lookup::Gte | Lookup::Lt | Lookup::Lte => true,
    }
}

fn json_lookup<'a>(item: &'a Value, path: &str) -> Option<&'a Value> {
    let mut value = item;
    for part in path.split('.') {
        value = value.as_object()?.get(part)?;
    }
    Some(value)
}

fn json_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null => None,
        Value::Object(_) | Value::Array(_) => None,
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FilterError {
    #[error("unknown filter: {0}")]
    UnknownFilter(String),

    #[error("unknown ordering field: {0}")]
    UnknownOrdering(String),

    #[error("invalid filter value for {field}, expected {expected}")]
    InvalidValue {
        field: String,
        expected: &'static str,
    },

    #[error("invalid lookup for field: {0}")]
    InvalidLookup(String),

    #[error("invalid model field: {0}")]
    InvalidModelField(String),
}

fn model_field<M: Model>(name: &str) -> Result<&'static FieldInfo, FilterError> {
    M::fields()
        .iter()
        .find(|field| field.rust_name == name || field.db_name == name)
        .ok_or_else(|| FilterError::InvalidModelField(name.to_string()))
}

fn parse_value(field: &FieldInfo, value: &str) -> Result<SqliteValue, FilterError> {
    match field.ty {
        FieldType::Integer => value
            .parse::<i64>()
            .map(SqliteValue::from)
            .map_err(|_| invalid_value(field, "integer")),
        FieldType::Text => Ok(SqliteValue::from(value)),
        FieldType::Boolean => parse_bool(value)
            .map(SqliteValue::from)
            .ok_or_else(|| invalid_value(field, "boolean")),
        FieldType::Real => value
            .parse::<f64>()
            .map(SqliteValue::from)
            .map_err(|_| invalid_value(field, "number")),
    }
}

fn validate_lookup(field: &FieldInfo, lookup: Lookup) -> Result<(), FilterError> {
    match lookup {
        Lookup::Exact => Ok(()),
        Lookup::Contains if field.ty == FieldType::Text => Ok(()),
        Lookup::Gt | Lookup::Gte | Lookup::Lt | Lookup::Lte
            if matches!(field.ty, FieldType::Integer | FieldType::Real) =>
        {
            Ok(())
        }
        _ => Err(FilterError::InvalidLookup(field.rust_name.to_string())),
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

fn parse_u32(field: &str, value: &str) -> Result<u32, FilterError> {
    value.parse::<u32>().map_err(|_| FilterError::InvalidValue {
        field: field.to_string(),
        expected: "positive integer",
    })
}

fn invalid_value(field: &FieldInfo, expected: &'static str) -> FilterError {
    FilterError::InvalidValue {
        field: field.rust_name.to_string(),
        expected,
    }
}
