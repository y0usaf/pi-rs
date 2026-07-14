//! Focused versioned root/action facade over the canonical kernel transaction.

pub(crate) fn install(lua: &mlua::Lua, pi: &mlua::Table) -> mlua::Result<()> {
    let kernel: mlua::Table = pi.get("kernel")?;
    let kernel_v1: mlua::Table = kernel.get("v1")?;
    let register: mlua::Function = kernel_v1.get("root")?;

    let v1 = lua.create_table()?;
    v1.set("api_version", 1_u32)?;
    v1.set(
        "register",
        lua.create_function(move |_, definition: mlua::Table| {
            let kind: String = definition.get("kind")?;
            if !matches!(kind.as_str(), "application" | "agent" | "frontend") {
                return Err(mlua::Error::runtime(
                    "roots.v1 supports application, agent, and frontend roots",
                ));
            }
            register.call::<()>(definition)
        })?,
    )?;
    v1.set("action", kernel_v1.get::<mlua::Function>("action")?)?;
    v1.set(
        "cancellation",
        kernel_v1.get::<mlua::Function>("cancellation")?,
    )?;
    v1.set("module", kernel_v1.get::<mlua::Table>("module")?)?;

    let roots = lua.create_table()?;
    roots.set("v1", v1)?;
    pi.set("roots", roots)
}
