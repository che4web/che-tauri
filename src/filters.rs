use std::{collections::HashMap, marker::PhantomData};

use che_orm::{
    ColumnRef, ColumnSchema, ColumnType, CompareOp, DatabaseValue, Expr, Model, OrderBy,
    OrderDirection, SelectQuery,
};
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
    M: Model,
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

    pub fn apply(
        &self,
        mut query: SelectQuery<M>,
        params: &HashMap<String, String>,
    ) -> ApiResult<SelectQuery<M>> {
        for (name, value) in params {
            match name.as_str() {
                "ordering" => {
                    query = self.apply_ordering(query, value)?;
                }
                "limit" => {
                    query = query.limit(u64::from(parse_u32("limit", value)?));
                }
                "offset" => {
                    query = query.offset(u64::from(parse_u32("offset", value)?));
                }
                name => {
                    let filter = self
                        .filter_by_query_name(name)
                        .ok_or_else(|| FilterError::UnknownFilter(name.to_string()))?;

                    if filter.remote_mode == RemoteFilterMode::RemoteOnly {
                        continue;
                    }

                    let field = model_field::<M>(filter.source)?;
                    validate_lookup(&field, filter.lookup)?;
                    let value = parse_value(&field, value)?;
                    let column =
                        Box::new(Expr::Column(ColumnRef::new(M::table_name(), field.name)));
                    let expr = if filter.lookup == Lookup::Contains {
                        let DatabaseValue::Text(value) = value else {
                            unreachable!()
                        };
                        Expr::Like {
                            left: column,
                            pattern: Box::new(Expr::Value(DatabaseValue::Text(format!(
                                "%{value}%"
                            )))),
                        }
                    } else {
                        let op = match filter.lookup {
                            Lookup::Exact => CompareOp::Eq,
                            Lookup::Gt => CompareOp::Gt,
                            Lookup::Gte => CompareOp::Gte,
                            Lookup::Lt => CompareOp::Lt,
                            Lookup::Lte => CompareOp::Lte,
                            Lookup::Contains => unreachable!(),
                        };
                        Expr::Compare {
                            left: column,
                            op,
                            right: Box::new(Expr::Value(value)),
                        }
                    };
                    query = query.filter(expr);
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

    fn apply_ordering(&self, query: SelectQuery<M>, value: &str) -> ApiResult<SelectQuery<M>> {
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
        let field = model_field::<M>(source)?;
        Ok(query.order_by(OrderBy {
            column: ColumnRef::new(M::table_name(), field.name),
            direction: if value.starts_with('-') {
                OrderDirection::Desc
            } else {
                OrderDirection::Asc
            },
        }))
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

fn model_field<M: Model>(name: &str) -> Result<ColumnSchema, FilterError> {
    M::schema()
        .columns
        .into_iter()
        .find(|field| field.field_name == name || field.name == name)
        .ok_or_else(|| FilterError::InvalidModelField(name.to_string()))
}

fn parse_value(field: &ColumnSchema, value: &str) -> Result<DatabaseValue, FilterError> {
    match field.column_type {
        ColumnType::Integer => value
            .parse::<i64>()
            .map(DatabaseValue::Integer)
            .map_err(|_| invalid_value(field, "integer")),
        ColumnType::Text => Ok(DatabaseValue::Text(value.to_string())),
        ColumnType::Uuid => che_orm::Uuid::parse_str(value)
            .map(DatabaseValue::Uuid)
            .map_err(|_| invalid_value(field, "UUID string")),
        ColumnType::Real => value
            .parse::<f64>()
            .ok()
            .filter(|value| value.is_finite())
            .map(DatabaseValue::Real)
            .ok_or_else(|| invalid_value(field, "finite number")),
        ColumnType::Boolean => parse_bool(value)
            .map(DatabaseValue::Boolean)
            .ok_or_else(|| invalid_value(field, "boolean")),
        ColumnType::DateTime => che_orm::time::OffsetDateTime::parse(
            value,
            &che_orm::time::format_description::well_known::Rfc3339,
        )
        .map(DatabaseValue::DateTime)
        .map_err(|_| invalid_value(field, "RFC 3339 timestamp")),
        ColumnType::Binary => Err(invalid_value(field, "binary filters are unsupported")),
    }
}

fn validate_lookup(field: &ColumnSchema, lookup: Lookup) -> Result<(), FilterError> {
    match lookup {
        Lookup::Exact => Ok(()),
        Lookup::Contains if field.column_type == ColumnType::Text => Ok(()),
        Lookup::Gt | Lookup::Gte | Lookup::Lt | Lookup::Lte
            if matches!(
                field.column_type,
                ColumnType::Integer | ColumnType::Real | ColumnType::DateTime
            ) =>
        {
            Ok(())
        }
        _ => Err(FilterError::InvalidLookup(field.field_name.to_string())),
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

fn invalid_value(field: &ColumnSchema, expected: &'static str) -> FilterError {
    FilterError::InvalidValue {
        field: field.field_name.to_string(),
        expected,
    }
}
