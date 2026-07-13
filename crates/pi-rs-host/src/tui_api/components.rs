use crate::bindings::rendered_lines;
use mlua::{AnyUserData, UserData, UserDataMethods};
use std::sync::{Arc, Mutex};

pub(crate) struct LuaText(pub(crate) Arc<pi_rs_tui::component::Text>);
impl UserData for LuaText {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("set_text", |_, this, text: String| {
            this.0.set_text(text);
            Ok(())
        });
    }
}

pub(crate) struct LuaInput(pub(crate) Arc<pi_rs_tui::input::Input>);
impl UserData for LuaInput {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("value", |_, this, ()| Ok(this.0.value()));
        methods.add_method("set_value", |_, this, value: String| {
            this.0.set_value(value);
            Ok(())
        });
        methods.add_method("set_focused", |_, this, focused: bool| {
            this.0.set_focused(focused);
            Ok(())
        });
        methods.add_method("input", |lua, this, data: String| {
            use pi_rs_tui::input::InputEvent;
            let event = lua.create_table()?;
            match this.0.handle_input(&data) {
                InputEvent::Changed(value) => {
                    event.set("kind", "changed")?;
                    event.set("value", value)?;
                }
                InputEvent::Submit(value) => {
                    event.set("kind", "submit")?;
                    event.set("value", value)?;
                }
                InputEvent::Cancel => event.set("kind", "cancel")?,
                InputEvent::None => event.set("kind", "none")?,
            }
            Ok(event)
        });
    }
}

pub(crate) struct LuaSettingsList(pub(crate) Arc<pi_rs_tui::settings_list::SettingsList>);
impl UserData for LuaSettingsList {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("selected", |lua, this, ()| {
            this.0
                .selected()
                .map(|item| {
                    let value = lua.create_table()?;
                    value.set("id", item.id)?;
                    value.set("label", item.label)?;
                    value.set("description", item.description)?;
                    value.set("current_value", item.current_value)?;
                    value.set("values", item.values)?;
                    Ok(value)
                })
                .transpose()
        });
        methods.add_method("update_value", |_, this, (id, value): (String, String)| {
            this.0.update_value(&id, value);
            Ok(())
        });
        methods.add_method("set_query", |_, this, query: String| {
            this.0.set_query(&query);
            Ok(())
        });
        methods.add_method("query", |_, this, ()| Ok(this.0.query()));
        methods.add_method("select_id", |_, this, id: String| {
            this.0.select_id(&id);
            Ok(())
        });
        methods.add_method("move_up", |_, this, ()| {
            this.0.move_up();
            Ok(())
        });
        methods.add_method("move_down", |_, this, ()| {
            this.0.move_down();
            Ok(())
        });
        methods.add_method("activate", |lua, this, ()| {
            use pi_rs_tui::settings_list::SettingsListAction;
            let action = lua.create_table()?;
            match this.0.activate() {
                SettingsListAction::Changed { id, value } => {
                    action.set("id", id)?;
                    action.set("value", value)?;
                }
                SettingsListAction::Submenu { id, current_value } => {
                    action.set("kind", "submenu")?;
                    action.set("id", id)?;
                    action.set("value", current_value)?;
                }
                SettingsListAction::Cancel => action.set("kind", "cancel")?,
                SettingsListAction::None => action.set("kind", "none")?,
            }
            Ok(action)
        });
        methods.add_method("input", |lua, this, data: String| {
            use pi_rs_tui::settings_list::SettingsListAction;
            let action = lua.create_table()?;
            match this.0.handle_input(&data) {
                SettingsListAction::Changed { id, value } => {
                    action.set("kind", "changed")?;
                    action.set("id", id)?;
                    action.set("value", value)?;
                }
                SettingsListAction::Submenu { id, current_value } => {
                    action.set("kind", "submenu")?;
                    action.set("id", id)?;
                    action.set("value", current_value)?;
                }
                SettingsListAction::Cancel => action.set("kind", "cancel")?,
                SettingsListAction::None => action.set("kind", "none")?,
            }
            Ok(action)
        });
    }
}

pub(crate) struct LuaSpacer(pub(crate) Arc<pi_rs_tui::spacer::Spacer>);
impl UserData for LuaSpacer {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("set_lines", |_, this, lines: usize| {
            this.0.set_lines(lines);
            Ok(())
        });
    }
}

pub(crate) struct LuaTruncatedText(pub(crate) Arc<pi_rs_tui::truncated_text::TruncatedText>);
impl UserData for LuaTruncatedText {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("render", |lua, this, width: usize| {
            rendered_lines(
                lua,
                pi_rs_tui::component::Component::render(&*this.0, width),
            )
        });
        methods.add_method("set_text", |_, this, text: String| {
            this.0.set_text(text);
            Ok(())
        });
    }
}

fn component_handle(value: &AnyUserData) -> mlua::Result<Arc<dyn pi_rs_tui::component::Component>> {
    if let Ok(component) = value.borrow::<LuaText>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaInput>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaSettingsList>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaSpacer>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaTruncatedText>() {
        return Ok(component.0.clone());
    }
    if let Ok(component) = value.borrow::<LuaBox>() {
        return Ok(component.component.clone());
    }
    Err(mlua::Error::external(
        "expected a pi.tui component userdata",
    ))
}

pub(crate) struct LuaBox {
    pub(crate) component: Arc<pi_rs_tui::box_component::BoxComponent>,
    pub(crate) background: Mutex<Option<mlua::RegistryKey>>,
}
impl UserData for LuaBox {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("add", |_, this, child: AnyUserData| {
            this.component.add(component_handle(&child)?);
            Ok(())
        });
        methods.add_method("remove", |_, this, child: AnyUserData| {
            let child = component_handle(&child)?;
            this.component.remove(&child);
            Ok(())
        });
        methods.add_method("clear", |_, this, ()| {
            this.component.clear();
            Ok(())
        });
        methods.add_method(
            "set_background",
            |lua, this, background: Option<mlua::Function>| {
                let key = background
                    .map(|function| lua.create_registry_value(function))
                    .transpose()?;
                let mut stored = this
                    .background
                    .lock()
                    .map_err(|_| mlua::Error::external("box background lock poisoned"))?;
                if let Some(old) = stored.take() {
                    lua.remove_registry_value(old)?;
                }
                *stored = key;
                Ok(())
            },
        );
        methods.add_method("render", |lua, this, width: usize| {
            let mut lines = pi_rs_tui::component::Component::render(&*this.component, width);
            let stored = this
                .background
                .lock()
                .map_err(|_| mlua::Error::external("box background lock poisoned"))?;
            if let Some(key) = stored.as_ref() {
                let background: mlua::Function = lua.registry_value(key)?;
                lines = lines
                    .into_iter()
                    .map(|line| background.call(line))
                    .collect::<mlua::Result<_>>()?;
            }
            rendered_lines(lua, lines)
        });
    }
}
