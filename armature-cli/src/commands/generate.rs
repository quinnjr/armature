//! Code generation commands.

use colored::Colorize;
use heck::ToSnakeCase;

use crate::error::{CliError, CliResult};
use crate::generators::{NameCases, ensure_dir, get_src_dir, update_mod_file, write_file};
use crate::templates::{
    ComponentData, ControllerData, DtoData, EntityData, ModelData, ModuleData, TemplateRegistry,
};

/// A single user-specified field parsed from a `--fields` spec.
struct FieldDef {
    snake: String,
    rust_type: &'static str,
}

/// Map a field type token (e.g. `string`, `i32`, `bool`) to a Rust type.
fn rust_type_for(spec: &str) -> &'static str {
    match spec.trim().to_lowercase().as_str() {
        "string" | "str" | "text" | "varchar" | "uuid" => "String",
        "int" | "integer" | "i32" => "i32",
        "i64" | "long" | "bigint" => "i64",
        "u32" => "u32",
        "u64" => "u64",
        "float" | "f64" | "double" | "decimal" => "f64",
        "f32" => "f32",
        "bool" | "boolean" => "bool",
        "date" | "datetime" | "timestamp" => "String",
        "json" => "serde_json::Value",
        _ => "String",
    }
}

/// Single source-of-truth table mapping a Rust type (as produced by
/// [`rust_type_for`]) to its Diesel type name and plain SQL column type.
///
/// Keeping both mappings in one table prevents the two from silently
/// diverging (e.g. missing arms like the `serde_json::Value` -> `JSONB` case).
const TYPE_MAPPINGS: &[(&str, &str, &str)] = &[
    // (rust_type, diesel_type, sql_type)
    ("String", "Text", "VARCHAR"),
    ("i32", "Integer", "INTEGER"),
    ("i64", "BigInt", "BIGINT"),
    ("u32", "BigInt", "BIGINT"),
    ("u64", "BigInt", "BIGINT"),
    ("f64", "Double", "DOUBLE PRECISION"),
    ("f32", "Double", "DOUBLE PRECISION"),
    ("bool", "Bool", "BOOLEAN"),
    ("serde_json::Value", "Text", "JSONB"),
];

/// Default Diesel type for a Rust type with no explicit mapping.
const DEFAULT_DIESEL_TYPE: &str = "Text";
/// Default SQL type for a Rust type with no explicit mapping.
const DEFAULT_SQL_TYPE: &str = "TEXT";

/// Map a Rust type to a Diesel SQL column type.
fn diesel_type_for(rust_type: &str) -> &'static str {
    TYPE_MAPPINGS
        .iter()
        .find(|(rt, _, _)| *rt == rust_type)
        .map(|(_, diesel, _)| *diesel)
        .unwrap_or(DEFAULT_DIESEL_TYPE)
}

/// Map a Rust type to a plain SQL column type (used for generated migrations).
fn sql_type_for(rust_type: &str) -> &'static str {
    TYPE_MAPPINGS
        .iter()
        .find(|(rt, _, _)| *rt == rust_type)
        .map(|(_, _, sql)| *sql)
        .unwrap_or(DEFAULT_SQL_TYPE)
}

/// Field names already declared by the base model struct / migration SQL
/// (see `MODEL_TEMPLATE` and `generate_migration` in `templates.rs`).
///
/// A user-supplied `--fields` entry that collides with one of these produces
/// a duplicate struct field / SQL column, so it must be rejected up front.
const RESERVED_FIELD_NAMES: &[&str] = &["id", "name", "created_at", "updated_at"];

/// Parse a `--fields` spec like `name:string,email:string` into field definitions.
///
/// Invalid or empty entries are skipped; a bare `name` (no type) defaults to `String`.
fn parse_fields(spec: Option<&str>) -> Vec<FieldDef> {
    let Some(spec) = spec else {
        return Vec::new();
    };
    spec.split(',')
        .filter_map(|pair| {
            let mut it = pair.splitn(2, ':');
            let name = it.next()?.trim();
            if name.is_empty() {
                return None;
            }
            let ty = it.next().unwrap_or("string");
            Some(FieldDef {
                snake: name.to_snake_case(),
                rust_type: rust_type_for(ty),
            })
        })
        .collect()
}

/// Reject `--fields` entries that collide with the reserved column/field
/// names already hardcoded into the base model template and migration SQL
/// (`id`, `name`, `created_at`, `updated_at`). Without this check, a field
/// like `--fields name:string` produces a struct/table with a duplicate
/// field and fails to compile/run.
fn check_reserved_field_names(fields: &[FieldDef]) -> CliResult<()> {
    for field in fields {
        if RESERVED_FIELD_NAMES.contains(&field.snake.as_str()) {
            return Err(CliError::InvalidArgument(format!(
                "field name '{}' collides with a column already provided by the base model template; choose a different name",
                field.snake
            )));
        }
    }
    Ok(())
}

/// Generate a controller.
pub async fn controller(name: &str, crud: bool, skip_tests: bool, auth: bool) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let controllers_dir = src_dir.join("controllers");
    ensure_dir(&controllers_dir)?;

    let templates = TemplateRegistry::new();

    // Determine base path (handle nested paths like "api/users")
    let base_path = if name.contains('/') {
        name.to_string()
    } else {
        names.kebab.clone()
    };

    // When --auth is set, attach an authentication guard to the controller.
    let guard_attr = if auth {
        "#[guard(AuthGuard)]\n".to_string()
    } else {
        String::new()
    };

    let data = ControllerData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
        base_path,
        guard_attr,
    };

    // Generate controller file
    let template_name = if crud {
        "controller_crud"
    } else {
        "controller"
    };
    let controller_content = templates
        .render(template_name, &data)
        .map_err(CliError::Template)?;

    let controller_file = controllers_dir.join(format!("{}.rs", names.snake));
    write_file(&controller_file, &controller_content, false)?;

    println!(
        "  {} {}",
        "CREATE".green().bold(),
        controller_file.display()
    );

    // Update mod.rs
    update_mod_file(&controllers_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        controllers_dir.join("mod.rs").display()
    );

    // Generate test file
    if !skip_tests {
        let test_content = templates
            .render("controller_test", &data)
            .map_err(CliError::Template)?;

        let tests_dir = controllers_dir.join("tests");
        ensure_dir(&tests_dir)?;

        let test_file = tests_dir.join(format!("{}_test.rs", names.snake));
        write_file(&test_file, &test_content, false)?;

        println!("  {} {}", "CREATE".green().bold(), test_file.display());
    }

    println!(
        "\n{} Generated {}Controller{}",
        "✓".green().bold(),
        names.pascal,
        if crud { " with CRUD endpoints" } else { "" }
    );

    Ok(())
}

/// Generate a module.
pub async fn module(
    name: &str,
    controllers: Option<&str>,
    providers: Option<&str>,
) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;

    let templates = TemplateRegistry::new();

    let controller_list: Vec<String> = controllers
        .map(|s| {
            s.split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let provider_list: Vec<String> = providers
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let data = ModuleData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        controllers: controller_list.iter().map(|c| c.to_string()).collect(),
        providers: provider_list.iter().map(|p| p.to_string()).collect(),
        controller_list: controller_list
            .iter()
            .map(|c| format!("{}Controller", heck::AsPascalCase(c)))
            .collect::<Vec<_>>()
            .join(", "),
        provider_list: provider_list
            .iter()
            .map(|p| format!("{}Service", heck::AsPascalCase(p)))
            .collect::<Vec<_>>()
            .join(", "),
    };

    let module_content = templates
        .render("module", &data)
        .map_err(CliError::Template)?;

    // Create module directory
    let module_dir = src_dir.join(&names.snake);
    ensure_dir(&module_dir)?;

    let module_file = module_dir.join("mod.rs");
    write_file(&module_file, &module_content, false)?;

    println!("  {} {}", "CREATE".green().bold(), module_file.display());

    // Update main mod.rs
    update_mod_file(&src_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        src_dir.join("mod.rs").display()
    );

    println!("\n{} Generated {}Module", "✓".green().bold(), names.pascal);

    Ok(())
}

/// Generate middleware.
pub async fn middleware(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("middleware", name, skip_tests).await
}

/// Generate a guard, choosing the implementation from `guard_type`.
///
/// `guard_type` is one of `custom`, `auth`, `role`, `permission`, `apikey`,
/// `ratelimit`. Each renders a distinct guard implementation.
pub async fn guard(name: &str, skip_tests: bool, guard_type: &str) -> CliResult<()> {
    let (template, test_template): (&str, Option<&str>) = match guard_type {
        "auth" => ("guard_auth", Some("guard_test")),
        "role" => ("guard_role", None),
        "permission" => ("guard_permission", None),
        "apikey" => ("guard_apikey", None),
        "ratelimit" => ("guard_ratelimit", None),
        // "custom" and anything else fall back to the generic guard.
        _ => ("guard", Some("guard_test")),
    };
    generate_component_templated("guards", "Guard", template, test_template, name, skip_tests).await
}

/// Generate a service.
pub async fn service(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("service", name, skip_tests).await
}

/// Generate a complete resource (controller + service + module).
pub async fn resource(name: &str, crud: bool) -> CliResult<()> {
    println!(
        "  {} Generating resource: {}",
        "→".cyan().bold(),
        name.cyan()
    );
    println!();

    // Generate service
    println!("  {} Generating service...", "1/3".dimmed());
    service(name, false).await?;
    println!();

    // Generate controller
    println!("  {} Generating controller...", "2/3".dimmed());
    controller(name, crud, false, false).await?;
    println!();

    // Generate module
    println!("  {} Generating module...", "3/3".dimmed());
    module(name, Some(name), Some(name)).await?;

    println!(
        "\n{} Resource {} generated successfully!",
        "✓".green().bold(),
        name.green()
    );
    println!(
        "  {} Don't forget to import the module in your main.rs",
        "→".yellow()
    );

    Ok(())
}

/// Generate a repository.
pub async fn repository(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("repository", name, skip_tests).await
}

/// Generate DTOs (Data Transfer Objects), optionally injecting `--fields`.
pub async fn dto(name: &str, fields: Option<&str>) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let dto_dir = src_dir.join("dto");
    ensure_dir(&dto_dir)?;

    let templates = TemplateRegistry::new();

    let parsed = parse_fields(fields);

    // Response struct: extra fields between `id` and the timestamps.
    let response_fields = parsed
        .iter()
        .map(|f| format!("    pub {}: {},\n", f.snake, f.rust_type))
        .collect::<String>();

    // Create request: validated fields (defaults to a single `name` field).
    let create_fields = if parsed.is_empty() {
        "    #[validate(length(min = 1, max = 255, message = \"Name must be between 1 and 255 characters\"))]\n    pub name: String,\n".to_string()
    } else {
        parsed
            .iter()
            .map(|f| {
                if f.rust_type == "String" {
                    format!(
                        "    #[validate(length(min = 1, message = \"{} is required\"))]\n    pub {}: {},\n",
                        f.snake, f.snake, f.rust_type
                    )
                } else {
                    format!("    pub {}: {},\n", f.snake, f.rust_type)
                }
            })
            .collect::<String>()
    };

    // Update request: every field optional.
    let update_fields = if parsed.is_empty() {
        "    pub name: Option<String>,\n".to_string()
    } else {
        parsed
            .iter()
            .map(|f| format!("    pub {}: Option<{}>,\n", f.snake, f.rust_type))
            .collect::<String>()
    };

    let data = DtoData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        response_fields,
        create_fields,
        update_fields,
    };

    let content = templates.render("dto", &data).map_err(CliError::Template)?;

    let file_path = dto_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&dto_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        dto_dir.join("mod.rs").display()
    );

    println!("\n{} Generated {}Dto", "✓".green().bold(), names.pascal);
    Ok(())
}

/// Generate a plain data model (see [`ModelData`]), optionally injecting
/// `--fields` and, when `migration` is set, a paired up/down SQL migration.
pub async fn model(name: &str, fields: Option<&str>, migration: bool) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let models_dir = src_dir.join("models");
    ensure_dir(&models_dir)?;

    let templates = TemplateRegistry::new();

    let parsed = parse_fields(fields);
    check_reserved_field_names(&parsed)?;
    let model_fields = parsed
        .iter()
        .map(|f| format!("    pub {}: {},\n", f.snake, f.rust_type))
        .collect::<String>();

    let data = ModelData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
        model_fields,
    };

    let content = templates
        .render("model", &data)
        .map_err(CliError::Template)?;

    let file_path = models_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&models_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        models_dir.join("mod.rs").display()
    );

    if migration {
        let (up_file, down_file) = generate_migration(&names, &parsed)?;
        println!("  {} {}", "CREATE".green().bold(), up_file.display());
        println!("  {} {}", "CREATE".green().bold(), down_file.display());
    }

    println!("\n{} Generated {}Model", "✓".green().bold(), names.pascal);
    Ok(())
}

/// Generate a Diesel-style up/down SQL migration for a model.
///
/// Mirrors the directory layout `diesel_migrations::embed_migrations!` expects
/// (see `docs/diesel-guide.md`): a `migrations/<timestamp>_create_<table>/`
/// directory at the project root containing `up.sql` and `down.sql`.
/// Returns the paths of the two files written.
fn generate_migration(
    names: &NameCases,
    fields: &[FieldDef],
) -> CliResult<(std::path::PathBuf, std::path::PathBuf)> {
    let src_dir = get_src_dir()?;
    let project_root = src_dir.parent().unwrap_or(&src_dir).to_path_buf();
    let table = format!("{}s", names.snake);

    let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
    let migration_dir = project_root
        .join("migrations")
        .join(format!("{}_create_{}", timestamp, table));
    ensure_dir(&migration_dir)?;

    let columns = fields
        .iter()
        .map(|f| format!("    {} {} NOT NULL,\n", f.snake, sql_type_for(f.rust_type)))
        .collect::<String>();

    let up_sql = format!(
        "-- Create {table} table\nCREATE TABLE {table} (\n    id BIGSERIAL PRIMARY KEY,\n    name VARCHAR NOT NULL,\n{columns}    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),\n    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\n);\n",
        table = table,
        columns = columns,
    );
    let down_sql = format!(
        "-- Drop {table} table\nDROP TABLE IF EXISTS {table};\n",
        table = table
    );

    let up_file = migration_dir.join("up.sql");
    let down_file = migration_dir.join("down.sql");
    // `migration_dir` is timestamp-named and freshly created by this call above,
    // so on a partial write it's always safe to remove wholesale rather than
    // leaving an orphaned up.sql with no down.sql behind.
    if let Err(e) =
        write_file(&up_file, &up_sql, false).and_then(|_| write_file(&down_file, &down_sql, false))
    {
        let _ = std::fs::remove_dir_all(&migration_dir); // best-effort cleanup of partial write
        return Err(e);
    }

    Ok((up_file, down_file))
}

/// Generate a WebSocket handler.
pub async fn websocket(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("websocket", name, skip_tests).await
}

/// Generate a GraphQL resolver.
pub async fn graphql_resolver(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("graphql_resolver", name, skip_tests).await
}

/// Generate a background job, choosing the implementation from `job_type`.
///
/// `job_type` is one of `async` (queue processor), `scheduled` (cron), or
/// `recurring` (fixed interval).
pub async fn job(name: &str, skip_tests: bool, job_type: &str) -> CliResult<()> {
    let (template, test_template): (&str, Option<&str>) = match job_type {
        "scheduled" => ("job_scheduled", None),
        "recurring" => ("job_recurring", None),
        // "async" (default) is a queue processor and ships with a test.
        _ => ("job", Some("job_test")),
    };
    generate_component_templated("jobs", "Job", template, test_template, name, skip_tests).await
}

/// Generate an event handler.
pub async fn event_handler(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("event_handler", name, skip_tests).await
}

/// Generate an interceptor.
pub async fn interceptor(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("interceptor", name, skip_tests).await
}

/// Generate a validation pipe.
pub async fn pipe(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("pipe", name, skip_tests).await
}

/// Generate an exception filter.
pub async fn exception_filter(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("exception_filter", name, skip_tests).await
}

/// Generate a configuration module.
pub async fn config(name: &str) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let config_dir = src_dir.join("config");
    ensure_dir(&config_dir)?;

    let templates = TemplateRegistry::new();

    let data = crate::templates::ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    let content = templates
        .render("config", &data)
        .map_err(CliError::Template)?;

    let file_path = config_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&config_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        config_dir.join("mod.rs").display()
    );

    println!("\n{} Generated {}Config", "✓".green().bold(), names.pascal);
    Ok(())
}

/// ORM type for entity generation.
#[derive(Debug, Clone, Copy, Default)]
pub enum OrmType {
    #[default]
    Generic,
    Diesel,
    SeaOrm,
    Prax,
}

impl std::str::FromStr for OrmType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "generic" | "none" => Ok(OrmType::Generic),
            "diesel" => Ok(OrmType::Diesel),
            "seaorm" | "sea-orm" | "sea_orm" => Ok(OrmType::SeaOrm),
            "prax" | "prax-orm" | "prax_orm" => Ok(OrmType::Prax),
            _ => Err(format!(
                "Unknown ORM type: {}. Valid options: generic, diesel, seaorm, prax",
                s
            )),
        }
    }
}

/// Generate a database entity.
pub async fn entity(name: &str) -> CliResult<()> {
    entity_with_orm(name, OrmType::Generic, None).await
}

/// Generate a database entity with specific ORM support, optionally injecting `--fields`.
pub async fn entity_with_orm(name: &str, orm: OrmType, fields: Option<&str>) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let entities_dir = src_dir.join("entities");
    ensure_dir(&entities_dir)?;

    let templates = TemplateRegistry::new();

    // The Prax ORM sources fields from `schema.prax`, so it uses the plain template.
    let content = if matches!(orm, OrmType::Prax) {
        let data = ComponentData {
            name_pascal: names.pascal.clone(),
            name_snake: names.snake.clone(),
            name_kebab: names.kebab.clone(),
        };
        templates
            .render("entity_prax", &data)
            .map_err(CliError::Template)?
    } else {
        let parsed = parse_fields(fields);
        let entity_fields = parsed
            .iter()
            .map(|f| format!("    pub {}: {},\n", f.snake, f.rust_type))
            .collect::<String>();
        let update_fields = parsed
            .iter()
            .map(|f| format!("    pub {}: Option<{}>,\n", f.snake, f.rust_type))
            .collect::<String>();
        let diesel_cols = parsed
            .iter()
            .map(|f| format!("        {} -> {},\n", f.snake, diesel_type_for(f.rust_type)))
            .collect::<String>();
        let seaorm_fields = parsed
            .iter()
            .map(|f| format!("        pub {}: {},\n", f.snake, f.rust_type))
            .collect::<String>();
        let data = EntityData {
            name_pascal: names.pascal.clone(),
            name_snake: names.snake.clone(),
            name_kebab: names.kebab.clone(),
            entity_fields: entity_fields.clone(),
            new_fields: entity_fields,
            update_fields,
            diesel_cols,
            seaorm_fields,
        };
        templates
            .render("entity", &data)
            .map_err(CliError::Template)?
    };

    let file_path = entities_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&entities_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        entities_dir.join("mod.rs").display()
    );

    let orm_name = match orm {
        OrmType::Generic => "generic",
        OrmType::Diesel => "Diesel",
        OrmType::SeaOrm => "SeaORM",
        OrmType::Prax => "Prax",
    };

    println!(
        "\n{} Generated {} entity ({})",
        "✓".green().bold(),
        names.pascal,
        orm_name.cyan()
    );
    Ok(())
}

/// Generate a Prax ORM schema file.
pub async fn prax_schema(name: &str) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let schema_dir = src_dir.parent().unwrap_or(&src_dir).to_path_buf();

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    let content = templates
        .render("prax_schema", &data)
        .map_err(CliError::Template)?;

    let file_path = schema_dir.join("schema.prax");
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());
    println!(
        "\n{} Generated Prax schema for {}",
        "✓".green().bold(),
        names.pascal
    );
    println!(
        "  {} Run {} to generate Rust code",
        "→".yellow(),
        "prax generate".cyan()
    );
    Ok(())
}

/// Generate a Prax ORM repository.
pub async fn prax_repository(name: &str, skip_tests: bool) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let repo_dir = src_dir.join("repositories");
    ensure_dir(&repo_dir)?;

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    // Generate repository
    let content = templates
        .render("prax_repository", &data)
        .map_err(CliError::Template)?;

    let file_path = repo_dir.join(format!("{}_repository.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&repo_dir, &format!("{}_repository", names.snake))?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        repo_dir.join("mod.rs").display()
    );

    // Generate test file
    if !skip_tests {
        let test_content = templates
            .render("prax_repository_test", &data)
            .map_err(CliError::Template)?;

        let tests_dir = repo_dir.join("tests");
        ensure_dir(&tests_dir)?;

        let test_file = tests_dir.join(format!("{}_repository_test.rs", names.snake));
        write_file(&test_file, &test_content, false)?;

        println!("  {} {}", "CREATE".green().bold(), test_file.display());
    }

    println!(
        "\n{} Generated {}Repository (Prax ORM)",
        "✓".green().bold(),
        names.pascal
    );
    Ok(())
}

/// Generate a complete Prax ORM module with entity, repository, and service.
pub async fn prax_module(name: &str) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    println!(
        "  {} Generating Prax ORM module: {}",
        "→".cyan().bold(),
        name.cyan()
    );
    println!();

    // 1. Generate schema file
    println!("  {} Generating Prax schema...", "1/5".dimmed());
    prax_schema(name).await?;
    println!();

    // 2. Generate entity
    println!("  {} Generating entity...", "2/5".dimmed());
    entity_with_orm(name, OrmType::Prax, None).await?;
    println!();

    // 3. Generate repository
    println!("  {} Generating repository...", "3/5".dimmed());
    prax_repository(name, false).await?;
    println!();

    // 4. Generate service
    println!("  {} Generating service...", "4/5".dimmed());
    service(name, false).await?;
    println!();

    // 5. Generate module file
    println!("  {} Generating module...", "5/5".dimmed());
    let module_dir = src_dir.join(&names.snake);
    ensure_dir(&module_dir)?;

    let module_content = templates
        .render("prax_module", &data)
        .map_err(CliError::Template)?;

    let module_file = module_dir.join("mod.rs");
    write_file(&module_file, &module_content, false)?;

    println!("  {} {}", "CREATE".green().bold(), module_file.display());

    update_mod_file(&src_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        src_dir.join("mod.rs").display()
    );

    println!(
        "\n{} Prax module {} generated successfully!",
        "✓".green().bold(),
        name.green()
    );
    println!();
    println!("  {} Next steps:", "💡".yellow());
    println!("    {} Add prax-armature to Cargo.toml:", "1.".dimmed());
    println!("       {}", r#"prax-armature = "0.4""#.cyan());
    println!("    {} Run Prax code generation:", "2.".dimmed());
    println!("       {}", "prax generate".cyan());
    println!("    {} Import the module in main.rs:", "3.".dimmed());
    println!(
        "       {}",
        format!("use {}::{}Module;", names.snake, names.pascal).cyan()
    );

    Ok(())
}

/// Generate a scheduled task.
pub async fn scheduler(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("scheduler", name, skip_tests).await
}

/// Generate a cache service.
pub async fn cache_service(name: &str, skip_tests: bool) -> CliResult<()> {
    generate_component("cache_service", name, skip_tests).await
}

/// Generate an API client.
pub async fn api_client(name: &str) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;
    let clients_dir = src_dir.join("clients");
    ensure_dir(&clients_dir)?;

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    let content = templates
        .render("api_client", &data)
        .map_err(CliError::Template)?;

    let file_path = clients_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&clients_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        clients_dir.join("mod.rs").display()
    );

    println!("\n{} Generated {}Client", "✓".green().bold(), names.pascal);
    Ok(())
}

/// Generate a health check controller.
pub async fn health_controller() -> CliResult<()> {
    let src_dir = get_src_dir()?;
    let controllers_dir = src_dir.join("controllers");
    ensure_dir(&controllers_dir)?;

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: "Health".to_string(),
        name_snake: "health".to_string(),
        name_kebab: "health".to_string(),
    };

    let content = templates
        .render("health_controller", &data)
        .map_err(CliError::Template)?;

    let file_path = controllers_dir.join("health.rs");
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&controllers_dir, "health")?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        controllers_dir.join("mod.rs").display()
    );

    println!("\n{} Generated HealthController", "✓".green().bold());
    Ok(())
}

/// Generate a component from an explicit template + directory + test template.
///
/// Used by generators whose implementation varies (e.g. guards by `--guard-type`,
/// jobs by `--job-type`). `test_template` is `None` when the variant ships no test.
async fn generate_component_templated(
    dir_name: &str,
    type_name: &str,
    template_name: &str,
    test_template: Option<&str>,
    name: &str,
    skip_tests: bool,
) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;

    let component_dir = src_dir.join(dir_name);
    ensure_dir(&component_dir)?;

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    let content = templates
        .render(template_name, &data)
        .map_err(CliError::Template)?;

    let file_path = component_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    update_mod_file(&component_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        component_dir.join("mod.rs").display()
    );

    if !skip_tests && let Some(test_template) = test_template {
        let test_content = templates
            .render(test_template, &data)
            .map_err(CliError::Template)?;

        let tests_dir = component_dir.join("tests");
        ensure_dir(&tests_dir)?;

        let test_file = tests_dir.join(format!("{}_test.rs", names.snake));
        write_file(&test_file, &test_content, false)?;

        println!("  {} {}", "CREATE".green().bold(), test_file.display());
    }

    println!(
        "\n{} Generated {}{}",
        "✓".green().bold(),
        names.pascal,
        type_name
    );

    Ok(())
}

/// Generic component generator for middleware, guards, services, and more.
async fn generate_component(component_type: &str, name: &str, skip_tests: bool) -> CliResult<()> {
    let names = NameCases::from(name);
    let src_dir = get_src_dir()?;

    let dir_name = match component_type {
        "middleware" => "middleware",
        "guard" => "guards",
        "service" => "services",
        "repository" => "repositories",
        "websocket" => "websockets",
        "graphql_resolver" => "graphql",
        "job" => "jobs",
        "event_handler" => "events",
        "interceptor" => "interceptors",
        "pipe" => "pipes",
        "exception_filter" => "filters",
        "scheduler" => "tasks",
        "cache_service" => "cache",
        _ => {
            return Err(CliError::InvalidArgument(format!(
                "Unknown component type: {}",
                component_type
            )));
        }
    };

    let component_dir = src_dir.join(dir_name);
    ensure_dir(&component_dir)?;

    let templates = TemplateRegistry::new();

    let data = ComponentData {
        name_pascal: names.pascal.clone(),
        name_snake: names.snake.clone(),
        name_kebab: names.kebab.clone(),
    };

    // Generate main file
    let content = templates
        .render(component_type, &data)
        .map_err(CliError::Template)?;

    let file_path = component_dir.join(format!("{}.rs", names.snake));
    write_file(&file_path, &content, false)?;

    println!("  {} {}", "CREATE".green().bold(), file_path.display());

    // Update mod.rs
    update_mod_file(&component_dir, &names.snake)?;
    println!(
        "  {} {}",
        "UPDATE".yellow().bold(),
        component_dir.join("mod.rs").display()
    );

    // Generate test file
    if !skip_tests {
        let test_template = format!("{}_test", component_type);
        let test_content = templates
            .render(&test_template, &data)
            .map_err(CliError::Template)?;

        let tests_dir = component_dir.join("tests");
        ensure_dir(&tests_dir)?;

        let test_file = tests_dir.join(format!("{}_test.rs", names.snake));
        write_file(&test_file, &test_content, false)?;

        println!("  {} {}", "CREATE".green().bold(), test_file.display());
    }

    let type_name = match component_type {
        "middleware" => "Middleware",
        "guard" => "Guard",
        "service" => "Service",
        "repository" => "Repository",
        "websocket" => "WebSocket",
        "graphql_resolver" => "Resolver",
        "job" => "Job",
        "event_handler" => "EventHandler",
        "interceptor" => "Interceptor",
        "pipe" => "Pipe",
        "exception_filter" => "ExceptionFilter",
        "scheduler" => "Task",
        "cache_service" => "CacheService",
        _ => "Component",
    };

    println!(
        "\n{} Generated {}{}",
        "✓".green().bold(),
        names.pascal,
        type_name
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_reserved_field_names_rejects_collisions_with_base_template() {
        // `MODEL_TEMPLATE` (templates.rs) hardcodes `id`, `name`, `created_at`,
        // and `updated_at`; user-supplied fields with these names must be
        // rejected instead of silently producing a struct/table with a
        // duplicate field.
        for reserved in RESERVED_FIELD_NAMES {
            let fields = parse_fields(Some(&format!("{reserved}:string")));
            let result = check_reserved_field_names(&fields);
            assert!(
                result.is_err(),
                "expected field name '{reserved}' to be rejected as reserved, got: {result:?}"
            );
            assert!(
                matches!(result, Err(CliError::InvalidArgument(_))),
                "expected a CliError::InvalidArgument for reserved field '{reserved}', got: {result:?}"
            );
        }
    }

    #[test]
    fn check_reserved_field_names_allows_non_colliding_names() {
        let fields = parse_fields(Some("email:string,age:i32,active:bool"));
        assert!(check_reserved_field_names(&fields).is_ok());
    }
}
