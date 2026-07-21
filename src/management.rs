use std::{fs, path::PathBuf};

use che_orm::{FieldType, Schema, SqliteBackend, diff_schemas, sqlite_migration_sql};
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
        let module = self
            .apps
            .find(app)
            .ok_or_else(|| format!("app is not installed: {app}"))?;

        let migrations_dir = app_migrations_dir(&apps_dir, app);
        fs::create_dir_all(&migrations_dir)?;

        let snapshot_path = migrations_dir.join("schema.json");
        let old_schema = Schema::load_or_empty(&snapshot_path)?;
        let new_schema = app_schema(module);
        let migration = diff_schemas(&old_schema, &new_schema);

        if migration.changes.is_empty() {
            println!("No schema changes detected for app {app}");
            return Ok(());
        }

        let sql = sqlite_migration_sql(&migration);
        let file_name = format!(
            "{:04}_{}.sql",
            next_migration_number(&migrations_dir)?,
            slugify(name)
        );
        let migration_path = migrations_dir.join(file_name);
        fs::write(&migration_path, format!("{sql}\n"))?;
        new_schema.save(snapshot_path)?;

        println!("Created {}", migration_path.display());

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
        let db = SqliteBackend::connect(&database_url).await?;

        match app {
            Some(app) => {
                validate_app_name(app)?;
                if self.apps.find(app).is_none() {
                    return Err(format!("app is not installed: {app}").into());
                }
                apply_app_migrations(&db, &apps_dir, app).await?;
            }
            None => {
                for app in self.apps.names() {
                    apply_app_migrations(&db, &apps_dir, app).await?;
                }
            }
        }

        Ok(())
    }

    fn generate_ts(&self, out: PathBuf) -> ManageResult<()> {
        let endpoints = self.api_endpoints();
        fs::create_dir_all(&out)?;
        fs::write(out.join("api_client.ts"), api_client_ts())?;
        fs::write(out.join("models.ts"), models_ts(&endpoints))?;
        fs::write(out.join("api.ts"), api_ts(&endpoints))?;

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

fn app_schema(module: &dyn crate::AppModule) -> Schema {
    let mut ctx = ModuleContext::new();
    module.init(&mut ctx);
    Schema::from_models(ctx.model_schemas().to_vec())
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
                ts_type(field.ty, field.nullable)
            ));
        }
        out.push_str("}\n\n");

        out.push_str(&format!(
            "export interface {}Update {{\n",
            endpoint.model_name
        ));
        for field in endpoint.fields.iter().filter(|field| !field.read_only) {
            out.push_str(&format!(
                "  {}?: {};\n",
                field.name,
                ts_type(field.ty, field.nullable)
            ));
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

fn ts_type(ty: FieldType, nullable: bool) -> String {
    let base = match ty {
        FieldType::Integer | FieldType::Real => "number",
        FieldType::Text => "string",
        FieldType::Boolean => "boolean",
    };
    if nullable {
        format!("{base} | null")
    } else {
        base.to_string()
    }
}

fn response_ts_type(field: &ApiField) -> String {
    match &field.related_model {
        Some(model) if field.nullable => format!("{model} | null"),
        Some(model) => model.clone(),
        None => ts_type(field.ty, field.nullable),
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
        request: { resource, action: "list", params: params ?? {} },
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
"#
}

async fn apply_app_migrations(
    db: &SqliteBackend,
    apps_dir: &std::path::Path,
    app: &str,
) -> ManageResult<()> {
    let migrations_dir = app_migrations_dir(apps_dir, app);
    for name in db
        .apply_migrations_dir_with_namespace(app, &migrations_dir)
        .await?
    {
        println!("Applied {app}: {name}");
    }
    Ok(())
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
        if path.extension().is_some_and(|extension| extension == "sql") {
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
pub mod serializers;

use che_tauri::{{AppModule, ModuleContext}};

pub fn module() -> {module_type} {{
    {module_type}
}}

pub struct {module_type};

impl AppModule for {module_type} {{
    fn name(&self) -> &'static str {{
        "{name}"
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

#[derive(Debug, Clone, Model)]
#[model(table = "{name}")]
pub struct {model} {{
    #[field(primary_key)]
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
