//! Markdown rendering of the configuration reference from the exported schemas: the
//! bootstrap JSON Schema (walked to its leaf keys, sorted) and the settings schema
//! (`{"settings": [...], "rules": [...]}`, in declaration order, grouped by category).
//!
//! Strict about shape, lenient about extras: a type, composition (`anyOf` / `oneOf` /
//! `allOf`), dangling `$ref`, non-object property or bootstrap leaf without a default
//! (secrets excepted) that the tables cannot describe is an error. Fields the tables do not
//! show (`format`, `minimum`, `examples`, `on_bad_row`, ...) are ignored on purpose.

use serde_json::Value as Json;

const TEMPLATE: &str = include_str!("configuration.template.md");
const BOOTSTRAP_SLOT: &str = "@BOOTSTRAP@";
const SETTINGS_SLOT: &str = "@SETTINGS@";
const RULES_SLOT: &str = "@RULES@";

const ENV_PREFIX: &str = "DESKHAVEN_";
const ENV_SEPARATOR: &str = "__";
const DEFS_PREFIX: &str = "#/$defs/";
/// Guards against self-referential schemas.
const MAX_DEPTH: usize = 16;
const COMPOSITION: [&str; 3] = ["anyOf", "oneOf", "allOf"];
/// The kind of a `writeOnly` value.
const SECRET: &str = "secret";

pub fn configuration(bootstrap: &Json, settings: &Json) -> Result<String, String> {
    let bootstrap = bootstrap_table(bootstrap)?;
    let settings_tables = settings_tables(settings)?;
    let rules = rules_list(settings)?;
    [
        (BOOTSTRAP_SLOT, bootstrap),
        (SETTINGS_SLOT, settings_tables),
        (RULES_SLOT, rules),
    ]
    .into_iter()
    .try_fold(TEMPLATE.to_owned(), |text, (slot, content)| {
        match text.matches(slot).count() {
            1 => Ok(text.replace(slot, content.trim_end())),
            n => Err(format!("the template has {n} {slot} slots, expected 1")),
        }
    })
}

// --- bootstrap ---

struct Leaf {
    key: String,
    kind: String,
    default: Json,
    description: String,
}

fn bootstrap_table(schema: &Json) -> Result<String, String> {
    let mut leaves = Vec::new();
    collect_leaves(schema, schema, "", 0, &mut leaves)?;
    leaves.sort_by(|a, b| a.key.cmp(&b.key));
    let rows = leaves.iter().map(|leaf| {
        [
            code(&leaf.key),
            code(&env_var(leaf)),
            leaf.kind.clone(),
            default_cell(&leaf.default),
            cell(&leaf.description),
        ]
    });
    Ok(table(
        [
            "Key",
            "Environment variable",
            "Type",
            "Default",
            "Description",
        ],
        rows,
    ))
}

fn collect_leaves(
    root: &Json,
    node: &Json,
    prefix: &str,
    depth: usize,
    out: &mut Vec<Leaf>,
) -> Result<(), String> {
    if depth > MAX_DEPTH {
        return Err(format!("bootstrap schema nests too deep at {prefix}"));
    }
    let properties = node
        .get("properties")
        .and_then(Json::as_object)
        .ok_or_else(|| format!("bootstrap schema: no properties at {prefix:?}"))?;
    for (name, property) in properties {
        let key = join(prefix, name);
        let target = resolve(root, checked(property, &key)?)?;
        if target.get("properties").is_some() {
            collect_leaves(root, checked(target, &key)?, &key, depth + 1, out)?;
            continue;
        }
        let kind = kind(root, property)?;
        let default = match (property.get("default"), kind.as_str()) {
            (Some(default), _) => default.clone(),
            // A secret may be left without one: it is unset until given.
            (None, SECRET) => Json::Null,
            (None, _) => return Err(format!("bootstrap schema: {key} has no default")),
        };
        out.push(Leaf {
            kind,
            default,
            description: description(property, target),
            key,
        });
    }
    Ok(())
}

/// A schema object without composition (`anyOf` / `oneOf` / `allOf`), which this
/// renderer cannot describe.
fn checked<'a>(node: &'a Json, key: &str) -> Result<&'a Json, String> {
    let object = node
        .as_object()
        .ok_or_else(|| format!("bootstrap schema: {key} is not a schema object"))?;
    match COMPOSITION.iter().find(|word| object.contains_key(**word)) {
        Some(word) => Err(format!("bootstrap schema: {key} uses {word}")),
        None => Ok(node),
    }
}

/// `$ref` target, or the node itself.
fn resolve<'a>(root: &'a Json, node: &'a Json) -> Result<&'a Json, String> {
    let Some(reference) = node.get("$ref").and_then(Json::as_str) else {
        return Ok(node);
    };
    reference
        .strip_prefix(DEFS_PREFIX)
        .and_then(|name| root.get("$defs")?.get(name))
        .ok_or_else(|| format!("bootstrap schema: unresolved $ref {reference}"))
}

fn ref_name(node: &Json) -> Option<&str> {
    node.get("$ref")?.as_str()?.strip_prefix(DEFS_PREFIX)
}

fn kind(root: &Json, node: &Json) -> Result<String, String> {
    let target = checked(resolve(root, node)?, "a value")?;
    let is_secret = [node, target]
        .iter()
        .any(|n| n.get("writeOnly") == Some(&Json::Bool(true)));
    if is_secret {
        return Ok(SECRET.to_owned());
    }
    if let Some(variants) = target.get("enum").and_then(Json::as_array) {
        return Ok(format!("one of {}", code_list(variants)?));
    }
    match target.get("type").and_then(Json::as_str) {
        Some("string") => Ok(ref_name(node).unwrap_or("string").to_ascii_lowercase()),
        Some("integer") => Ok("integer".to_owned()),
        Some("boolean") => Ok("bool".to_owned()),
        Some("object") => Ok("table".to_owned()),
        Some("array") => {
            let items = target
                .get("items")
                .ok_or("bootstrap schema: array without items")?;
            Ok(format!("list of {}", kind(root, items)?))
        }
        other => Err(format!("bootstrap schema: unknown type {other:?}")),
    }
}

fn description(node: &Json, target: &Json) -> String {
    [node, target]
        .iter()
        .find_map(|n| n.get("description").and_then(Json::as_str))
        .unwrap_or_default()
        .to_owned()
}

/// `listen.client_api` -> `DESKHAVEN_LISTEN__CLIENT_API`; a table takes one more segment
/// per entry (`DESKHAVEN_SETTINGS__<KEY>`).
fn env_var(leaf: &Leaf) -> String {
    let base = format!(
        "{ENV_PREFIX}{}",
        leaf.key.to_ascii_uppercase().replace('.', ENV_SEPARATOR)
    );
    match leaf.kind.as_str() {
        "table" => format!("{base}{ENV_SEPARATOR}<KEY>"),
        _ => base,
    }
}

// --- settings ---

fn entries<'a>(settings: &'a Json, field: &str) -> Result<&'a Vec<Json>, String> {
    settings
        .get(field)
        .and_then(Json::as_array)
        .ok_or_else(|| format!("settings schema: no {field} list"))
}

fn text<'a>(entry: &'a Json, field: &str) -> Result<&'a str, String> {
    entry
        .get(field)
        .and_then(Json::as_str)
        .ok_or_else(|| format!("settings schema: an entry without {field}: {entry}"))
}

fn settings_tables(settings: &Json) -> Result<String, String> {
    let entries = entries(settings, "settings")?;
    let mut categories: Vec<(&str, Vec<[String; 4]>)> = Vec::new();
    for entry in entries {
        let category = text(entry, "category")?;
        let row = setting_row(entry)?;
        match categories.iter_mut().find(|(name, _)| *name == category) {
            Some((_, rows)) => rows.push(row),
            None => categories.push((category, vec![row])),
        }
    }
    Ok(categories
        .into_iter()
        .map(|(category, rows)| {
            let header = ["Key", "Allowed values", "Default", "Description"];
            format!("### {category}\n\n{}", table(header, rows.into_iter()))
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

fn setting_row(entry: &Json) -> Result<[String; 4], String> {
    let default = match entry.get("sensitive") {
        Some(Json::Bool(true)) => "unset".to_owned(),
        _ => default_cell(entry.get("default").unwrap_or(&Json::Null)),
    };
    Ok([
        code(text(entry, "key")?),
        allowed(entry)?,
        default,
        cell(text(entry, "description")?),
    ])
}

fn allowed(entry: &Json) -> Result<String, String> {
    let field = |name: &str| entry.get(name).ok_or_else(|| missing(entry, name));
    match text(entry, "type")? {
        "bool" => Ok("`true`, `false`".to_owned()),
        "u32" | "duration" => {
            let range = format!(
                "{}–{}",
                code(&scalar(field("min")?)),
                code(&scalar(field("max")?))
            );
            match entry.get("zero_allowed") {
                Some(Json::Bool(true)) => Ok(format!("{range}, or `0`")),
                _ => Ok(range),
            }
        }
        "enum" => code_list(
            field("variants")?
                .as_array()
                .ok_or("variants: not a list")?,
        ),
        "string" => {
            let shape = match entry.get("format").and_then(Json::as_str) {
                Some("host") => "host name",
                Some("email") => "email address",
                _ => "text",
            };
            Ok(format!(
                "{shape}, at most {} characters",
                field("max_length")?
            ))
        }
        "secret" => Ok(format!(
            "secret, at most {} characters",
            field("max_length")?
        )),
        "host_port_list" => Ok(format!(
            "list of {}, at most {}",
            code(&scalar(field("item")?)),
            field("max_items")?
        )),
        other => Err(format!("settings schema: unknown type `{other}`")),
    }
}

fn missing(entry: &Json, field: &str) -> String {
    format!("settings schema: {} has no {field}", entry["key"])
}

fn rules_list(settings: &Json) -> Result<String, String> {
    entries(settings, "rules")?
        .iter()
        .map(|rule| Ok(format!("- {}\n", text(rule, "message")?)))
        .collect()
}

// --- markdown ---

fn table<const N: usize>(header: [&str; N], rows: impl Iterator<Item = [String; N]>) -> String {
    let line = |cells: &[String]| format!("| {} |\n", cells.join(" | "));
    let header: Vec<String> = header.iter().map(|h| (*h).to_owned()).collect();
    let rule = vec!["---".to_owned(); N];
    let mut out = line(&header) + &line(&rule);
    rows.for_each(|row| out.push_str(&line(&row)));
    out
}

/// Text for a table cell: one line, `|` escaped.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "\\|")
}

fn code(text: &str) -> String {
    format!("`{}`", cell(text))
}

fn code_list(values: &[Json]) -> Result<String, String> {
    let items: Vec<String> = values.iter().map(|v| code(&scalar(v))).collect();
    match items.is_empty() {
        true => Err("an empty list of values".to_owned()),
        false => Ok(items.join(", ")),
    }
}

/// A string without quotes, anything else as compact JSON.
fn scalar(value: &Json) -> String {
    match value {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

fn default_cell(value: &Json) -> String {
    match value {
        Json::Null => "unset".to_owned(),
        Json::String(s) if s.is_empty() => "empty".to_owned(),
        Json::Array(items) if items.is_empty() => "empty".to_owned(),
        Json::Object(map) if map.is_empty() => "empty".to_owned(),
        other => code(&scalar(other)),
    }
}

fn join(prefix: &str, key: &str) -> String {
    match prefix {
        "" => key.to_owned(),
        _ => format!("{prefix}.{key}"),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// A bootstrap schema with one property `x`, plus a `Secret` definition.
    fn with_property(property: Json) -> Json {
        json!({
            "properties": { "x": property },
            "$defs": { "Secret": { "type": "string", "writeOnly": true } },
        })
    }

    fn rendered(property: Json) -> Result<String, String> {
        bootstrap_table(&with_property(property))
    }

    #[test]
    fn known_leaf_shapes_render() {
        let row = rendered(json!({"type": "integer", "default": 3, "minimum": 0})).unwrap();
        assert!(
            row.contains("| `x` | `DESKHAVEN_X` | integer | `3` |"),
            "{row}"
        );
        let secret = rendered(json!({"$ref": "#/$defs/Secret"})).unwrap();
        assert!(secret.contains("| secret | unset |"), "{secret}");
    }

    #[test]
    fn unknown_bootstrap_shapes_are_errors() {
        let shapes = [
            json!({"anyOf": [{"type": "string"}, {"type": "null"}], "default": ""}),
            json!({"oneOf": [{"type": "string"}], "default": ""}),
            json!({"type": "string", "allOf": [{"minLength": 1}], "default": ""}),
            json!({"type": ["string", "null"], "default": ""}),
            json!({"type": "number", "default": 1.5}),
            json!({"$ref": "#/$defs/Missing", "default": ""}),
            json!(true),
            json!("string"),
        ];
        for shape in shapes {
            assert!(rendered(shape.clone()).is_err(), "accepted {shape}");
        }
    }

    #[test]
    fn a_leaf_without_a_default_is_an_error_unless_it_is_a_secret() {
        let error = rendered(json!({"type": "string"})).unwrap_err();
        assert!(error.contains("no default"), "{error}");
        assert!(rendered(json!({"$ref": "#/$defs/Secret"})).is_ok());
    }
}
