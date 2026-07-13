//! Exact-version Lua module declaration, resolution, and cache ownership.

use crate::runtime_registry::{current_source, registry_table, set_current_source};

fn module_key(name: &str, version: &str) -> String {
    format!("{name}\0{version}")
}

fn validate_module_identity(name: &str, version: &str) -> mlua::Result<()> {
    if name.trim().is_empty() {
        return Err(mlua::Error::runtime(
            "module name must be a non-empty string",
        ));
    }
    if version.trim().is_empty() {
        return Err(mlua::Error::runtime(
            "module version must be a non-empty string",
        ));
    }
    Ok(())
}

/// Resolve one exact module version. Factories receive only their declared
/// dependencies, sorted by alias before resolution; the cache is VM-wide and
/// source identity is restored around every factory call.
fn require_module(lua: &mlua::Lua, name: &str, version: &str) -> mlua::Result<mlua::Value> {
    validate_module_identity(name, version)?;
    let registry = registry_table(lua)?;
    let modules: mlua::Table = registry.get("modules")?;
    let key = module_key(name, version);
    let Some(entry) = modules.get::<Option<mlua::Table>>(key.as_str())? else {
        return Err(mlua::Error::runtime(format!(
            "module {name:?} version {version:?} is not defined (required by {})",
            current_source(lua)
        )));
    };
    let state: String = entry.get("state")?;
    if state == "loaded" {
        return entry.get("value");
    }
    let stack: mlua::Table = registry.get("module_stack")?;
    if state == "loading" {
        let mut chain = stack
            .sequence_values::<String>()
            .collect::<mlua::Result<Vec<_>>>()?;
        chain.push(format!("{name}@{version}"));
        return Err(mlua::Error::runtime(format!(
            "module dependency cycle: {}",
            chain.join(" -> ")
        )));
    }

    entry.set("state", "loading")?;
    stack.push(format!("{name}@{version}"))?;
    let source: String = entry.get("source")?;
    let previous_source = current_source(lua);
    set_current_source(lua, &source);
    let result = (|| {
        let declarations: mlua::Table = entry.get("dependencies")?;
        let mut aliases = declarations
            .pairs::<String, mlua::Table>()
            .map(|pair| pair.map(|(alias, _)| alias))
            .collect::<mlua::Result<Vec<_>>>()?;
        aliases.sort();
        let dependencies = lua.create_table()?;
        for alias in aliases {
            let dependency: mlua::Table = declarations.get(alias.as_str())?;
            let dependency_name: String = dependency.get("name")?;
            let dependency_version: String = dependency.get("version")?;
            let value = require_module(lua, &dependency_name, &dependency_version)?;
            dependencies.set(alias, value)?;
        }
        let factory: mlua::Function = entry.get("factory")?;
        let value: mlua::Value = factory.call(dependencies)?;
        if value.is_nil() {
            return Err(mlua::Error::runtime(format!(
                "module {name}@{version} factory returned nil"
            )));
        }
        Ok(value)
    })();
    set_current_source(lua, &previous_source);
    let stack_len = stack.raw_len();
    if stack_len > 0 {
        stack.raw_set(stack_len, mlua::Nil)?;
    }
    match result {
        Ok(value) => {
            entry.set("value", value.clone())?;
            entry.set("state", "loaded")?;
            Ok(value)
        }
        Err(error) => {
            entry.set("state", "defined")?;
            entry.set("value", mlua::Nil)?;
            Err(mlua::Error::runtime(format!(
                "module {name}@{version} from {source}: {error}"
            )))
        }
    }
}

/// Install and return the one module table shared by `pi.module` and
/// `pi.kernel.v1.module`.
pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<mlua::Table> {
    // Deterministic, exact-version Lua modules. A module factory receives only
    // its declared dependency aliases; definitions and imports are identical
    // for embedded and file-backed sources. Package transport is deliberately
    // outside this mechanism: modules are values after their package is loaded.
    let module_api = lua.create_table()?;
    module_api.set(
        "define",
        lua.create_function(|lua, definition: mlua::Table| {
            let name: String = definition.get("name")?;
            let version: String = definition.get("version")?;
            validate_module_identity(&name, &version)?;
            let factory = definition
                .get::<Option<mlua::Function>>("factory")?
                .ok_or_else(|| mlua::Error::runtime("module factory must be a function"))?;
            let dependencies = definition
                .get::<Option<mlua::Table>>("dependencies")?
                .unwrap_or(lua.create_table()?);

            // Copy + validate declarations so later caller mutation cannot alter
            // dependency resolution. Aliases are sorted to make diagnostics stable.
            let mut declared = dependencies
                .pairs::<String, mlua::Table>()
                .collect::<mlua::Result<Vec<_>>>()?;
            declared.sort_by(|(left, _), (right, _)| left.cmp(right));
            let copied_dependencies = lua.create_table()?;
            for (alias, dependency) in declared {
                if alias.trim().is_empty() {
                    return Err(mlua::Error::runtime(
                        "module dependency alias must be a non-empty string",
                    ));
                }
                let dependency_name: String = dependency.get("name")?;
                let dependency_version: String = dependency.get("version")?;
                validate_module_identity(&dependency_name, &dependency_version)?;
                let copied = lua.create_table()?;
                copied.set("name", dependency_name)?;
                copied.set("version", dependency_version)?;
                copied_dependencies.set(alias, copied)?;
            }

            let registry = registry_table(lua)?;
            let modules: mlua::Table = registry.get("modules")?;
            let key = module_key(&name, &version);
            if let Some(existing) = modules.get::<Option<mlua::Table>>(key.as_str())? {
                let existing_source: String = existing.get("source")?;
                return Err(mlua::Error::runtime(format!(
                    "module {name}@{version} is already defined by {existing_source}"
                )));
            }
            let entry = lua.create_table()?;
            entry.set("name", name.as_str())?;
            entry.set("version", version.as_str())?;
            entry.set("source", current_source(lua))?;
            entry.set(
                "scope",
                crate::kernel_api::scope_for_current_entry(lua)?.get(),
            )?;
            entry.set("dependencies", copied_dependencies)?;
            entry.set("factory", factory)?;
            entry.set("state", "defined")?;
            modules.set(key.as_str(), entry)?;
            let order: mlua::Table = registry.get("module_order")?;
            order.push(key)?;
            Ok(())
        })?,
    )?;
    module_api.set(
        "require",
        lua.create_function(|lua, (name, version): (String, String)| {
            require_module(lua, &name, &version)
        })?,
    )?;
    module_api.set(
        "list",
        lua.create_function(|lua, ()| {
            let registry = registry_table(lua)?;
            let modules: mlua::Table = registry.get("modules")?;
            let order: mlua::Table = registry.get("module_order")?;
            let result = lua.create_table()?;
            for key in order.sequence_values::<String>() {
                let key = key?;
                let Some(entry) = modules.get::<Option<mlua::Table>>(key.as_str())? else {
                    continue;
                };
                let item = lua.create_table()?;
                item.set("name", entry.get::<String>("name")?)?;
                item.set("version", entry.get::<String>("version")?)?;
                item.set("source", entry.get::<String>("source")?)?;
                item.set("state", entry.get::<String>("state")?)?;
                result.push(item)?;
            }
            Ok(result)
        })?,
    )?;
    pi.set("module", module_api.clone())?;
    Ok(module_api)
}
