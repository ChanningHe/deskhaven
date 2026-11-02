//! Facts about key paths read from the JSON Schema: secrets (`writeOnly`),
//! lists (`type: array`) and text (`type: string`). Walks `$ref`, `anyOf` /
//! `oneOf` / `allOf`, `items`, `additionalProperties` and `properties`, so
//! wrappers like `Option<Secret>` or `Vec<Secret>` are found too.

use std::collections::BTreeSet;

use serde_json::Value as Json;

/// Guards against self-referential schemas.
const MAX_DEPTH: usize = 32;
/// Path segment standing for "any list item / map value".
const ITEM: &str = "*";
const DEFS_PREFIX: &str = "#/$defs/";

#[derive(Debug, Default)]
pub(crate) struct SchemaFacts {
    secrets: BTreeSet<String>,
    lists: BTreeSet<String>,
    texts: BTreeSet<String>,
}

impl SchemaFacts {
    pub(crate) fn from_schema(schema: &Json) -> Self {
        let mut facts = Self::default();
        facts.visit(schema, schema, "", 0);
        facts
    }

    /// A secret path or anything below one.
    pub(crate) fn is_secret(&self, path: &str) -> bool {
        self.secrets.iter().any(|secret| {
            path == secret
                || path
                    .strip_prefix(secret.as_str())
                    .is_some_and(|rest| rest.starts_with('.'))
        })
    }

    pub(crate) fn is_list(&self, path: &str) -> bool {
        self.lists.contains(path)
    }

    pub(crate) fn is_text(&self, path: &str) -> bool {
        self.texts.contains(path)
    }

    fn visit(&mut self, root: &Json, node: &Json, path: &str, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        if node.get("writeOnly") == Some(&Json::Bool(true)) {
            self.secrets.insert(secret_scope(path));
        }
        if has_type(node, "array") {
            self.lists.insert(path.to_owned());
        }
        if has_type(node, "string") {
            self.texts.insert(path.to_owned());
        }
        if let Some(target) = resolve(root, node) {
            self.visit(root, target, path, depth + 1);
        }
        for key in ["anyOf", "oneOf", "allOf"] {
            for sub in node.get(key).and_then(Json::as_array).into_iter().flatten() {
                self.visit(root, sub, path, depth + 1);
            }
        }
        let item_path = join(path, ITEM);
        for key in ["items", "additionalProperties"] {
            if let Some(sub) = node.get(key).filter(|sub| sub.is_object()) {
                self.visit(root, sub, &item_path, depth + 1);
            }
        }
        for (key, sub) in node
            .get("properties")
            .and_then(Json::as_object)
            .into_iter()
            .flatten()
        {
            self.visit(root, sub, &join(path, key), depth + 1);
        }
    }
}

fn resolve<'a>(root: &'a Json, node: &Json) -> Option<&'a Json> {
    let name = node.get("$ref")?.as_str()?.strip_prefix(DEFS_PREFIX)?;
    root.get("$defs")?.get(name)
}

/// `type` may be a single name or a list (e.g. `["array", "null"]`).
fn has_type(node: &Json, name: &str) -> bool {
    match node.get("type") {
        Some(Json::String(t)) => t == name,
        Some(Json::Array(types)) => types.iter().any(|t| t == name),
        _ => false,
    }
}

/// A secret inside a list or map makes the whole container secret.
fn secret_scope(path: &str) -> String {
    let segments: Vec<&str> = path.split('.').take_while(|s| *s != ITEM).collect();
    segments.join(".")
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_owned()
    } else {
        format!("{prefix}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Secret;
    use schemars::JsonSchema;

    #[allow(dead_code)]
    #[derive(JsonSchema)]
    struct Inner {
        s: Secret,
        port: u16,
    }

    #[allow(dead_code)]
    #[derive(JsonSchema)]
    struct Probe {
        a: Option<Secret>,
        b: Vec<Secret>,
        nested: Inner,
        list: Option<Vec<String>>,
        text: Option<String>,
        n: u16,
    }

    fn facts() -> SchemaFacts {
        SchemaFacts::from_schema(&schemars::schema_for!(Probe).to_value())
    }

    #[test]
    fn finds_wrapped_and_nested_secrets() {
        let facts = facts();
        let expected = ["a", "b", "nested.s"].map(String::from);
        assert_eq!(facts.secrets, BTreeSet::from(expected));
        assert!(facts.is_secret("b.0") && facts.is_secret("nested.s"));
        assert!(!facts.is_secret("nested.port") && !facts.is_secret("n"));
    }

    #[test]
    fn list_and_text_come_from_schema_type() {
        let facts = facts();
        assert!(facts.is_list("b") && facts.is_list("list"));
        assert!(!facts.is_list("text") && !facts.is_list("n"));
        assert!(facts.is_text("text") && facts.is_text("a") && facts.is_text("nested.s"));
        assert!(!facts.is_text("n") && !facts.is_text("list"));
    }
}
