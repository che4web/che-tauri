use std::{fs, path::PathBuf};

use che_orm::{
    ColumnType, Migration, MigrationGraph, MigrationId, ProjectState, operations_from_changes,
    render_migration_rust,
};
use clap::{Parser, Subcommand};

use crate::{ApiEndpoint, ApiField, AppConfig, InstalledApps, ModuleContext};

type ManageResult<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug, Parser)]
#[command(name = "manage")]
#[command(about = "Project management commands for che-tauri applications")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Startapp {
        name: String,

        #[arg(long, default_value = "src/apps")]
        apps_dir: PathBuf,
    },
    Makemigrations {
        app: String,

        #[arg(long, default_value = "src/apps")]
        apps_dir: PathBuf,

        #[arg(long, default_value = "auto")]
        name: String,
    },
    Migrate {
        app: Option<String>,

        #[arg(long, default_value = "src/apps")]
        apps_dir: PathBuf,

        #[arg(long, default_value = "app.toml")]
        config: PathBuf,

        #[arg(long)]
        database_url: Option<String>,
    },
    GenerateTs {
        #[arg(long, default_value = "../src/generated")]
        out: PathBuf,
    },
}

pub struct Management {
    apps: InstalledApps,
}

impl Management {
    pub fn new(apps: InstalledApps) -> Self {
        Self { apps }
    }

    pub async fn run(self) -> ManageResult<()> {
        self.run_from(Cli::parse()).await
    }

    async fn run_from(self, cli: Cli) -> ManageResult<()> {
        match cli.command {
            Command::Startapp { name, apps_dir } => startapp(&name, apps_dir)?,
            Command::Makemigrations {
                app,
                apps_dir,
                name,
            } => self.makemigrations(&app, apps_dir, &name)?,
            Command::Migrate {
                app,
                apps_dir,
                config,
                database_url,
            } => {
                self.migrate(app.as_deref(), apps_dir, config, database_url)
                    .await?
            }
            Command::GenerateTs { out } => self.generate_ts(out)?,
        }

        Ok(())
    }

    fn makemigrations(&self, app: &str, apps_dir: PathBuf, name: &str) -> ManageResult<()> {
        validate_app_name(app)?;
        self.apps
            .find(app)
            .ok_or_else(|| format!("app is not installed: {app}"))?;

        let migrations_dir = app_migrations_dir(&apps_dir, app);
        fs::create_dir_all(&migrations_dir)?;

        let graph = MigrationGraph::new(
            self.apps
                .iter()
                .flat_map(|module| module.migrations())
                .collect(),
        )?;
        graph.validate_heads()?;
        for path in migration_files(&migrations_dir)? {
            let name = path.file_stem().unwrap().to_string_lossy();
            if !graph
                .ordered()
                .any(|migration| migration.id.app == app && migration.id.name == name)
            {
                return Err(format!("migration {app}:{name} is not registered; register AppModule::migrations() and rebuild manage").into());
            }
        }
        let old_state = graph.replay(ProjectState::default())?;
        let mut desired = self.apps.migration_state()?;
        for object in old_state.objects() {
            desired.add_schema_object(object.clone())?;
        }
        let changes = old_state
            .diff(&desired)?
            .into_iter()
            // Shared sync tables have the logical owner che_sync, but their
            // operations must be shipped in an installed application's history.
            .filter(|change| change.app() == app || change.app() == "che_sync")
            .collect::<Vec<_>>();

        if changes.is_empty() {
            println!("No schema changes detected for app {app}");
            return Ok(());
        }

        let file_name = format!(
            "{:04}_{}",
            next_migration_number(&migrations_dir)?,
            slugify(name)
        );
        let dependencies = self
            .apps
            .names()
            .flat_map(|app| graph.app_heads(app))
            .map(|migration| migration.id.clone())
            .collect();
        let migration = Migration::new(
            MigrationId {
                app: app.to_string(),
                name: file_name.clone(),
            },
            dependencies,
            operations_from_changes(changes),
        );
        let migration_path = migrations_dir.join(format!("{file_name}.rs"));
        fs::write(&migration_path, render_migration_rust(&migration))?;
        update_migrations_mod(&migrations_dir)?;

        println!("Created {}", migration_path.display());
        println!(
            "Register migrations::migrations() in AppModule::migrations() and rebuild manage before the next command."
        );

        Ok(())
    }

    async fn migrate(
        &self,
        app: Option<&str>,
        apps_dir: PathBuf,
        config: PathBuf,
        database_url: Option<String>,
    ) -> ManageResult<()> {
        let database_url = match database_url {
            Some(database_url) => database_url,
            None => AppConfig::from_file(config)?.database.url,
        };
        let db = crate::state::connect_database(&database_url).await?;

        if let Some(app) = app {
            validate_app_name(app)?;
            if self.apps.find(app).is_none() {
                return Err(format!("app is not installed: {app}").into());
            }
        }

        let migrations = self
            .apps
            .iter()
            .flat_map(|module| module.migrations())
            .collect::<Vec<_>>();
        let graph = MigrationGraph::new(migrations)?;
        graph.validate_heads()?;
        for module in self
            .apps
            .iter()
            .filter(|module| app.is_none_or(|app| app == module.name()))
        {
            for path in migration_files(&app_migrations_dir(&apps_dir, module.name()))? {
                let name = path.file_stem().unwrap().to_string_lossy();
                if !graph
                    .ordered()
                    .any(|migration| migration.id.app == module.name() && migration.id.name == name)
                {
                    return Err(format!(
                        "migration {}:{name} is not registered; rebuild manage",
                        module.name()
                    )
                    .into());
                }
            }
        }
        let target = app.and_then(|app| {
            graph
                .app_heads(app)
                .first()
                .map(|migration| migration.id.clone())
        });
        if app.is_some() && target.is_none() {
            return Ok(());
        }
        let applied = db.migrate_to(graph, target).await?;
        println!("Applied {applied} operation migrations");
        Ok(())
    }

    fn generate_ts(&self, out: PathBuf) -> ManageResult<()> {
        let endpoints = self.api_endpoints();
        fs::create_dir_all(&out)?;
        fs::write(out.join("api_client.ts"), api_client_ts())?;
        fs::write(out.join("models.ts"), models_ts(&endpoints))?;
        fs::write(out.join("api.ts"), api_ts(&endpoints))?;
        #[cfg(feature = "sync")]
        fs::write(out.join("sync.ts"), crate::sync::TYPESCRIPT_CLIENT)?;

        println!("Generated TypeScript Tauri API in {}", out.display());
        Ok(())
    }

    fn api_endpoints(&self) -> Vec<ApiEndpoint> {
        let mut endpoints = Vec::new();
        for module in self.apps.iter() {
            let mut ctx = ModuleContext::new();
            module.init(&mut ctx);
            endpoints.extend(ctx.api_endpoints().iter().cloned());
        }
        endpoints.sort_by(|left, right| left.model_name.cmp(&right.model_name));
        endpoints
    }
}

fn models_ts(endpoints: &[ApiEndpoint]) -> String {
    let mut out = String::from("import type { ListParams } from \"./api_client\";\n\n");

    for endpoint in endpoints {
        out.push_str(&format!("export interface {} {{\n", endpoint.model_name));
        for field in endpoint.fields.iter().filter(|field| !field.write_only) {
            out.push_str(&format!("  {}: {};\n", field.name, response_ts_type(field)));
        }
        out.push_str("}\n\n");

        out.push_str(&format!(
            "export interface {}Create {{\n",
            endpoint.model_name
        ));
        for field in endpoint.fields.iter().filter(|field| !field.read_only) {
            out.push_str(&format!(
                "  {}{}: {};\n",
                field.name,
                optional_marker(field),
                field_ts_type(field)
            ));
        }
        out.push_str("}\n\n");

        out.push_str(&format!(
            "export interface {}Update {{\n",
            endpoint.model_name
        ));
        for field in endpoint.fields.iter().filter(|field| !field.read_only) {
            out.push_str(&format!("  {}?: {};\n", field.name, field_ts_type(field)));
        }
        out.push_str("}\n\n");

        out.push_str(&format!(
            "export interface {}ListParams extends ListParams {{\n",
            endpoint.model_name
        ));
        for filter in &endpoint.filters {
            out.push_str(&format!(
                "  {}?: {};\n",
                filter.name,
                ts_type(filter.ty, filter.nullable)
            ));
        }
        out.push_str("}\n\n");
    }

    out
}

fn api_ts(endpoints: &[ApiEndpoint]) -> String {
    let mut out = String::from("import { createModelApi } from \"./api_client\";\n");

    if endpoints.is_empty() {
        out.push('\n');
        return out;
    }

    out.push_str("import type {\n");
    for endpoint in endpoints {
        out.push_str(&format!("  {},\n", endpoint.model_name));
        out.push_str(&format!("  {}Create,\n", endpoint.model_name));
        out.push_str(&format!("  {}Update,\n", endpoint.model_name));
        out.push_str(&format!("  {}ListParams,\n", endpoint.model_name));
    }
    out.push_str("} from \"./models\";\n\n");

    for endpoint in endpoints {
        out.push_str(&format!(
            "export const {}Api = createModelApi<{}, {}Create, {}Update, {}ListParams>(\"{}\");\n",
            lower_first(&endpoint.model_name),
            endpoint.model_name,
            endpoint.model_name,
            endpoint.model_name,
            endpoint.model_name,
            endpoint.resource
        ));
    }

    out
}

fn optional_marker(field: &ApiField) -> &'static str {
    if field.required && !field.has_default {
        ""
    } else {
        "?"
    }
}

fn ts_type(ty: ColumnType, nullable: bool) -> String {
    let base = match ty {
        ColumnType::Integer | ColumnType::Real => "number",
        ColumnType::Text | ColumnType::DateTime | ColumnType::Uuid => "string",
        ColumnType::Boolean => "boolean",
        ColumnType::Binary => "number[]",
    };
    if nullable {
        format!("{base} | null")
    } else {
        base.to_string()
    }
}

fn response_ts_type(field: &ApiField) -> String {
    if let Some(ts_type) = &field.ts_type {
        return maybe_nullable(ts_type.clone(), field.nullable);
    }

    match &field.related_model {
        Some(model) if field.nullable => format!("{model} | null"),
        Some(model) => model.clone(),
        None => ts_type(field.ty, field.nullable),
    }
}

fn field_ts_type(field: &ApiField) -> String {
    if let Some(ts_type) = &field.input_ts_type {
        return maybe_nullable(ts_type.clone(), field.nullable);
    }

    if let Some(ts_type) = &field.ts_type {
        return maybe_nullable(ts_type.clone(), field.nullable);
    }

    if field.related_model.is_some() {
        return if field.nullable {
            "unknown | null".to_string()
        } else {
            "unknown".to_string()
        };
    }

    ts_type(field.ty, field.nullable)
}

fn maybe_nullable(base: String, nullable: bool) -> String {
    if nullable {
        format!("{base} | null")
    } else {
        base
    }
}

fn lower_first(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn api_client_ts() -> &'static str {
    r#"import { invoke } from "@tauri-apps/api/core";

export interface BaseEntity {
  id?: number;
}

export interface ListParams {
  ordering?: string;
  limit?: number;
  offset?: number;
  [key: string]: string | number | boolean | null | undefined;
}

export interface PaginatedResponse<T> {
  count: number;
  results: T[];
}

export interface ApiError {
  code: string;
  detail: string;
}

export interface ModelApi<
  T extends BaseEntity,
  CreateDTO = Partial<T>,
  UpdateDTO = Partial<T>,
  Params extends ListParams = ListParams,
> {
  list: (params?: Params) => Promise<PaginatedResponse<T>>;
  retrieve: (id: number) => Promise<T>;
  create: (payload: CreateDTO) => Promise<T>;
  update: (id: number, payload: UpdateDTO) => Promise<T>;
  remove: (id: number) => Promise<{ deleted: true }>;
}

export function createModelApi<
  T extends BaseEntity,
  CreateDTO = Partial<T>,
  UpdateDTO = Partial<T>,
  Params extends ListParams = ListParams,
>(resource: string): ModelApi<T, CreateDTO, UpdateDTO, Params> {
  return {
    list(params) {
      return invoke<PaginatedResponse<T>>("che_api", {
        request: { resource, action: "list", params: serializeListParams(params) },
      });
    },

    retrieve(id) {
      return invoke<T>("che_api", {
        request: { resource, action: "retrieve", id },
      });
    },

    create(payload) {
      return invoke<T>("che_api", {
        request: { resource, action: "create", payload },
      });
    },

    update(id, payload) {
      return invoke<T>("che_api", {
        request: { resource, action: "update", id, payload },
      });
    },

    remove(id) {
      return invoke<{ deleted: true }>("che_api", {
        request: { resource, action: "delete", id },
      });
    },
  };
}

function serializeListParams(params?: ListParams) {
  const serialized: Record<string, string> = {};

  for (const [key, value] of Object.entries(params ?? {})) {
    if (value !== null && value !== undefined) {
      serialized[key] = String(value);
    }
  }

  return serialized;
}
"#
}

fn startapp(name: &str, apps_dir: PathBuf) -> ManageResult<()> {
    validate_app_name(name)?;

    let app_dir = apps_dir.join(name);
    if app_dir.exists() {
        return Err(format!("app already exists: {}", app_dir.display()).into());
    }

    fs::create_dir_all(&app_dir)?;
    fs::write(app_dir.join("mod.rs"), mod_template(name))?;
    fs::write(app_dir.join("models.rs"), models_template(name))?;
    fs::write(app_dir.join("serializers.rs"), serializers_template(name))?;
    fs::write(app_dir.join("filters.rs"), filters_template(name))?;
    fs::create_dir(app_dir.join("migrations"))?;
    update_migrations_mod(&app_dir.join("migrations"))?;

    update_apps_mod(&apps_dir, name)?;

    println!("created app {}", app_dir.display());
    println!("add it to apps::installed_apps(): .add({name}::module())");

    Ok(())
}

fn update_apps_mod(apps_dir: &std::path::Path, name: &str) -> ManageResult<()> {
    fs::create_dir_all(apps_dir)?;
    let mod_path = apps_dir.join("mod.rs");
    let line = format!("pub mod {name};");
    let mut content = fs::read_to_string(&mod_path).unwrap_or_default();

    if !content.lines().any(|existing| existing.trim() == line) {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&line);
        content.push('\n');
        fs::write(mod_path, content)?;
    }

    Ok(())
}

fn app_migrations_dir(apps_dir: &std::path::Path, app: &str) -> PathBuf {
    apps_dir.join(app).join("migrations")
}

fn migration_files(migrations_dir: &std::path::Path) -> ManageResult<Vec<PathBuf>> {
    if !migrations_dir.exists() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    for entry in fs::read_dir(migrations_dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|extension| extension == "rs")
            && path.file_name().is_some_and(|name| name != "mod.rs")
        {
            files.push(path);
        }
    }
    Ok(files)
}

fn next_migration_number(migrations_dir: &std::path::Path) -> ManageResult<u32> {
    let max = migration_files(migrations_dir)?
        .iter()
        .filter_map(|path| path.file_name()?.to_str()?.get(0..4)?.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    Ok(max + 1)
}

fn update_migrations_mod(dir: &std::path::Path) -> ManageResult<()> {
    let mut files = migration_files(dir)?;
    files.sort();
    let mut source = String::new();
    for (index, path) in files.iter().enumerate() {
        source.push_str(&format!(
            "#[path = {:?}]\nmod migration_{index};\n",
            path.file_name().unwrap().to_string_lossy()
        ));
    }
    source.push_str("\npub fn migrations() -> Vec<che_orm::Migration> {\n    vec![\n");
    for index in 0..files.len() {
        source.push_str(&format!("        migration_{index}::migration(),\n"));
    }
    source.push_str("    ]\n}\n");
    fs::write(dir.join("mod.rs"), source)?;
    Ok(())
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "auto".to_string()
    } else {
        slug
    }
}

fn validate_app_name(name: &str) -> ManageResult<()> {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return Err("app name cannot be empty".into());
    };
    if !first.is_ascii_lowercase() {
        return Err("app name must start with a lowercase ascii letter".into());
    }
    if !chars.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_') {
        return Err(
            "app name must contain only lowercase ascii letters, digits, and underscores".into(),
        );
    }
    Ok(())
}

fn mod_template(name: &str) -> String {
    let module_type = format!("{}Module", plural_camel(name));
    let model = singular_camel(name);
    format!(
        r#"pub mod filters;
pub mod models;
pub mod migrations;
pub mod serializers;

use che_tauri::{{AppModule, ModuleContext}};

pub fn module() -> {module_type} {{
    {module_type}
}}

pub struct {module_type};

impl AppModule for {module_type} {{
    fn migrations(&self) -> Vec<che_orm::Migration> {{
        migrations::migrations()
    }}

    fn name(&self) -> &'static str {{
        "{name}"
    }}

    fn schema(&self) -> che_orm::SchemaSet {{
        che_orm::SchemaSet::new().model::<models::{model}>()
    }}

    fn init(&self, ctx: &mut ModuleContext) {{
        ctx.resource::<models::{model}>(
            "{name}",
            serializers::{fn_name}_serializer(),
            filters::{fn_name}_filterset(),
        );
    }}
}}
"#,
        fn_name = singular_name(name)
    )
}

fn models_template(name: &str) -> String {
    let model = singular_camel(name);
    format!(
        r#"use che_orm::Model;

#[derive(Debug, Clone, serde::Serialize, Model)]
#[orm(table = "{name}")]
pub struct {model} {{
    #[orm(primary_key)]
    pub id: i64,

    pub name: String,
}}
"#
    )
}

fn serializers_template(name: &str) -> String {
    let model = singular_camel(name);
    let fn_name = singular_name(name);
    format!(
        r#"use che_tauri::{{Field, ModelSerializer}};

use super::models::{model};

static {const_name}_FIELDS: &[Field] = &[
    Field::new("id").read_only(),
    Field::new("name"),
];

pub fn {fn_name}_serializer() -> ModelSerializer<{model}> {{
    ModelSerializer::new({const_name}_FIELDS)
}}
"#,
        const_name = name.to_ascii_uppercase()
    )
}

fn filters_template(name: &str) -> String {
    let model = singular_camel(name);
    let fn_name = singular_name(name);
    format!(
        r#"use che_tauri::{{Filter, FilterSet}};

use super::models::{model};

static {const_name}_FILTERS: &[Filter] = &[
    Filter::exact("name"),
    Filter::contains("name"),
];

pub fn {fn_name}_filterset() -> FilterSet<{model}> {{
    FilterSet::new({const_name}_FILTERS)
}}
"#,
        const_name = name.to_ascii_uppercase()
    )
}

fn singular_name(name: &str) -> String {
    if let Some(stem) = name.strip_suffix("ies") {
        format!("{stem}y")
    } else if name.ends_with("ss") {
        name.to_string()
    } else {
        name.strip_suffix('s').unwrap_or(name).to_string()
    }
}

fn singular_camel(name: &str) -> String {
    camel_case(&singular_name(name))
}

fn plural_camel(name: &str) -> String {
    camel_case(name)
}

fn camel_case(name: &str) -> String {
    name.split('_')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppModule, Field, FilterSet, ModelSerializer};
    use che_orm::{Database, Model};

    #[derive(Model, serde::Serialize)]
    #[orm(table = "notes")]
    struct Note {
        #[orm(primary_key)]
        id: i64,
        title: String,
        data: Vec<u8>,
        #[serde(with = "che_orm::time::serde::rfc3339")]
        created_at: che_orm::time::OffsetDateTime,
    }

    struct Notes {
        compiled: bool,
    }
    impl AppModule for Notes {
        fn name(&self) -> &'static str {
            "notes"
        }
        fn schema(&self) -> che_orm::SchemaSet {
            che_orm::SchemaSet::new().model::<Note>()
        }
        fn init(&self, ctx: &mut ModuleContext) {
            static FIELDS: &[Field] = &[
                Field::new("id").read_only(),
                Field::new("title"),
                Field::new("data"),
                Field::new("created_at"),
            ];
            ctx.resource::<Note>("notes", ModelSerializer::new(FIELDS), FilterSet::new(&[]));
        }
        fn migrations(&self) -> Vec<Migration> {
            if !self.compiled {
                return Vec::new();
            }
            let desired = ProjectState::from_app_schema("notes", &self.schema());
            vec![Migration::new(
                MigrationId {
                    app: "notes".into(),
                    name: "0001_auto".into(),
                },
                vec![],
                operations_from_changes(ProjectState::default().diff(&desired).unwrap()),
            )]
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "che-tauri-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[tokio::test]
    async fn operation_migrations_generate_register_and_apply_once() {
        let temp = TempDir::new();
        let apps_dir = temp.0.join("apps");
        let uncompiled = Management::new(InstalledApps::new().add(Notes { compiled: false }));
        uncompiled
            .makemigrations("notes", apps_dir.clone(), "auto")
            .unwrap();
        let source = fs::read_to_string(apps_dir.join("notes/migrations/0001_auto.rs")).unwrap();
        assert!(source.contains("MigrationOperation::CreateModel"));
        assert!(
            uncompiled
                .makemigrations("notes", apps_dir.clone(), "auto")
                .unwrap_err()
                .to_string()
                .contains("not registered")
        );

        let compiled = Management::new(InstalledApps::new().add(Notes { compiled: true }));
        compiled
            .makemigrations("notes", apps_dir.clone(), "auto")
            .unwrap();
        let db_path = temp.0.join("db.sqlite");
        for _ in 0..2 {
            compiled
                .migrate(
                    Some("notes"),
                    apps_dir.clone(),
                    temp.0.join("unused.toml"),
                    Some(format!("sqlite://{}?mode=rwc", db_path.display())),
                )
                .await
                .unwrap();
        }
        let db = Database::connect(db_path.to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(db.count::<Note>().await.unwrap(), 0);
        let models = models_ts(&compiled.api_endpoints());
        assert!(models.contains("data: number[];"));
        assert!(models.contains("created_at: string;"));
    }

    #[test]
    fn startapp_uses_current_model_attributes_and_registers_migrations() {
        let temp = TempDir::new();
        startapp("users", temp.0.clone()).unwrap();
        let model = fs::read_to_string(temp.0.join("users/models.rs")).unwrap();
        assert!(model.contains("serde::Serialize"));
        assert!(model.contains("#[orm(table = \"users\")]"));
        assert!(model.contains("#[orm(primary_key)]"));
        let module = fs::read_to_string(temp.0.join("users/mod.rs")).unwrap();
        assert!(module.contains("fn schema(&self) -> che_orm::SchemaSet"));
        assert!(module.contains(".model::<models::User>()"));
        assert!(
            fs::read_to_string(temp.0.join("users/mod.rs"))
                .unwrap()
                .contains("migrations::migrations()")
        );
    }
    #[test]
    fn makemigrations_reads_schema_without_initializing_resources() {
        struct SchemaOnly;
        impl AppModule for SchemaOnly {
            fn name(&self) -> &'static str {
                "notes"
            }
            fn schema(&self) -> che_orm::SchemaSet {
                che_orm::SchemaSet::new().model::<Note>()
            }
            fn init(&self, _ctx: &mut ModuleContext) {
                panic!("makemigrations ran init");
            }
        }
        let temp = TempDir::new();
        Management::new(InstalledApps::new().add(SchemaOnly))
            .makemigrations("notes", temp.0.clone(), "initial")
            .unwrap();
        let source = fs::read_to_string(temp.0.join("notes/migrations/0001_initial.rs")).unwrap();
        assert!(source.contains("CreateModel"));
        assert!(source.contains("notes"));
    }

    #[cfg(feature = "sync")]
    #[test]
    fn sync_migrations_include_shared_runtime_tables_without_running_init() {
        #[derive(che_orm::Model)]
        #[orm(table = "sync_tasks", sync, sync_name = "tasks.task")]
        struct Task {
            #[orm(primary_key)]
            id: che_orm::Uuid,
            title: String,
        }
        struct Tasks;
        impl AppModule for Tasks {
            fn name(&self) -> &'static str {
                "tasks"
            }
            fn schema(&self) -> che_orm::SchemaSet {
                che_orm::SchemaSet::new().client_model::<Task>()
            }
            fn init(&self, _ctx: &mut ModuleContext) {
                panic!("makemigrations ran init");
            }
        }
        let temp = TempDir::new();
        Management::new(InstalledApps::new().add(Tasks))
            .makemigrations("tasks", temp.0.clone(), "initial")
            .unwrap();
        let source = fs::read_to_string(temp.0.join("tasks/migrations/0001_initial.rs")).unwrap();
        for table in [
            "sync_tasks",
            "_sync_meta",
            "_sync_server_sync_tasks",
            "_sync_outbox",
        ] {
            assert!(source.contains(table), "missing migration table {table}");
        }
    }
}
