//! Conversions between the editor's JSON value model and Lua values.
//!
//! The C# host exposes JSON `null` as a unique sentinel table. Plugins compare
//! against `hexal.json.null`; the same convention is preserved here.
//!
//! SPDX-License-Identifier: GPL-3.0-only
//! (C) Navid Momtahen - 2026

use mlua::{Lua, Result as LuaResult, Table, Value as LuaValue};

pub use crate::util::json::Value;

/// Converts a Lua value into the host JSON model.
pub fn from_lua(value: &LuaValue, null: &LuaValue) -> Value {
    match value {
        LuaValue::Nil => Value::Null,
        LuaValue::Boolean(value) => Value::Bool(*value),
        LuaValue::Integer(value) => Value::Number(*value as f64),
        LuaValue::Number(value) => Value::Number(*value),
        LuaValue::String(value) => Value::String(value.to_string_lossy()),
        LuaValue::Table(table) => {
            if let LuaValue::Table(sentinel) = null {
                if table == sentinel {
                    return Value::Null;
                }
            }
            table_to_value(table, null)
        }
        LuaValue::LightUserData(_) => Value::Null,
        LuaValue::Function(_) | LuaValue::Thread(_) | LuaValue::UserData(_) => Value::Null,
        _ => Value::Null,
    }
}

fn table_to_value(table: &Table, null: &LuaValue) -> Value {
    let mut entries: Vec<(LuaValue, LuaValue)> = Vec::new();
    for pair in table.clone().pairs::<LuaValue, LuaValue>() {
        let Ok((key, value)) = pair else {
            return Value::Null;
        };
        entries.push((key, value));
    }

    let mut numeric: Vec<(i64, LuaValue)> = Vec::with_capacity(entries.len());
    let mut all_numeric = !entries.is_empty();
    for (key, value) in &entries {
        match numeric_key(key) {
            Some(index) => numeric.push((index, value.clone())),
            None => {
                all_numeric = false;
                break;
            }
        }
    }

    if all_numeric {
        numeric.sort_by(|left, right| left.0.cmp(&right.0));
        if numeric
            .iter()
            .enumerate()
            .all(|(index, (key, _))| *key == index as i64 + 1)
        {
            return Value::Array(
                numeric
                    .into_iter()
                    .map(|(_, value)| from_lua(&value, null))
                    .collect(),
            );
        }
    }

    let mut object: Vec<(String, Value)> = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let key = match key {
            LuaValue::String(value) => value.to_string_lossy(),
            LuaValue::Integer(value) => value.to_string(),
            LuaValue::Number(value) => {
                if value.fract() == 0.0 {
                    (value as i64).to_string()
                } else {
                    value.to_string()
                }
            }
            LuaValue::Boolean(value) => value.to_string(),
            _ => continue,
        };
        if let Some(existing) = object.iter_mut().find(|(name, _)| *name == key) {
            existing.1 = from_lua(&value, null);
        } else {
            object.push((key, from_lua(&value, null)));
        }
    }
    Value::Object(object)
}

fn numeric_key(key: &LuaValue) -> Option<i64> {
    match key {
        LuaValue::Integer(value) => Some(*value),
        LuaValue::Number(value) if value.fract() == 0.0 => Some(*value as i64),
        _ => None,
    }
}

/// Converts a host JSON value into a Lua value.
pub fn to_lua(lua: &Lua, value: &Value, null: &LuaValue) -> LuaResult<LuaValue> {
    Ok(match value {
        Value::Null => null.clone(),
        Value::Bool(value) => LuaValue::Boolean(*value),
        Value::Number(value) => {
            if value.fract() == 0.0 && value.is_finite() {
                LuaValue::Integer(*value as i64)
            } else {
                LuaValue::Number(*value)
            }
        }
        Value::String(value) => LuaValue::String(lua.create_string(value)?),
        Value::Array(values) => {
            let table = lua.create_table_with_capacity(values.len(), 0)?;
            for (index, value) in values.iter().enumerate() {
                table.raw_set(index + 1, to_lua(lua, value, null)?)?;
            }
            LuaValue::Table(table)
        }
        Value::Object(entries) => {
            let table = lua.create_table_with_capacity(0, entries.len())?;
            for (key, value) in entries {
                table.raw_set(key.as_str(), to_lua(lua, value, null)?)?;
            }
            LuaValue::Table(table)
        }
    })
}

pub fn get_i64(value: Option<&Value>, fallback: i64) -> i64 {
    value.and_then(Value::as_i64).unwrap_or(fallback)
}

pub fn get_bool(value: Option<&Value>, fallback: bool) -> bool {
    value.and_then(Value::as_bool).unwrap_or(fallback)
}

pub fn get_str(value: Option<&Value>, fallback: &str) -> String {
    value
        .and_then(Value::as_str)
        .unwrap_or(fallback)
        .to_string()
}

pub fn push(array: &mut Value, value: Value) {
    if let Value::Array(values) = array {
        values.push(value);
    }
}

pub fn as_array(value: &Value) -> &[Value] {
    value.as_array().unwrap_or(&[])
}

pub fn as_object(value: &Value) -> &[(String, Value)] {
    value.as_object().unwrap_or(&[])
}
