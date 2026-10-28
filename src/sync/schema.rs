//! Serialization schemas for che-sync protocol values used in typed invoke DTOs.
//! These are schema proxies, not a second runtime serialization implementation.
//! Attach with `#[schemars(with = "Vec<che_tauri::sync::schema::Conflict>")]` to
//! a field whose actual type is `Vec<che_sync::protocol::Conflict>`.
#![allow(dead_code)]
use che_orm::Uuid;
use schemars::JsonSchema;
use std::collections::BTreeMap;

#[derive(JsonSchema)]
#[schemars(rename = "SyncValue")]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Value {
    Integer(String),
    Real(f64),
    Text(String),
    Binary(Vec<u8>),
    Boolean(bool),
    DateTime(String),
    Uuid(Uuid),
    Null,
}

#[derive(JsonSchema)]
#[schemars(rename = "SyncEntity")]
pub struct Entity {
    model: String,
    id: Uuid,
    schema_version: u32,
    revision: String,
    #[serde(default)]
    epoch: Option<Uuid>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    deleted_at: Option<String>,
    record: Option<BTreeMap<String, Value>>,
}

#[derive(JsonSchema)]
#[schemars(rename = "SyncConflict")]
pub struct Conflict {
    model: String,
    id: Uuid,
    base: Option<BTreeMap<String, Value>>,
    local: Option<BTreeMap<String, Value>>,
    remote: Entity,
    fields: Vec<String>,
    reason: Option<String>,
    #[serde(default)]
    retryable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    acknowledgement: Option<Entity>,
}
