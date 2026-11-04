//! `GET /strategy-catalog`: the client options the editor knows, generated from the client
//! source (see `deskhaven_core::strategies::catalog`).

use deskhaven_core::strategies::catalog::{self, CatalogOption, Group, OptionKind, Text};
use serde::Serialize;
use utoipa::ToSchema;

use axum::Json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionKindView {
    /// `Y` or `N`.
    Bool,
    /// One of `choices`.
    Choice,
    /// A whole number within `min` / `max`.
    Number,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum OptionGroupView {
    Security,
    Permissions,
    Display,
    Network,
    Recording,
    Presets,
    Other,
}

impl From<Group> for OptionGroupView {
    fn from(g: Group) -> Self {
        match g {
            Group::Security => Self::Security,
            Group::Permissions => Self::Permissions,
            Group::Display => Self::Display,
            Group::Network => Self::Network,
            Group::Recording => Self::Recording,
            Group::Presets => Self::Presets,
            Group::Other => Self::Other,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct TextView {
    pub en: String,
    pub zh: String,
}

impl From<Text> for TextView {
    fn from(t: Text) -> Self {
        Self {
            en: t.en.to_owned(),
            zh: t.zh.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct ChoiceView {
    pub value: String,
    pub label: TextView,
}

/// One known client option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CatalogOptionView {
    pub key: String,
    pub kind: OptionKindView,
    pub group: OptionGroupView,
    /// What the client does when the option is unset; absent when the source does not say.
    pub default: Option<String>,
    /// A credential: never shown back once saved, masked in audit rows.
    pub secret: bool,
    /// The allowed values of a `choice` option.
    pub choices: Vec<ChoiceView>,
    pub min: Option<i64>,
    pub max: Option<i64>,
    pub label: TextView,
    pub description: TextView,
}

impl From<&CatalogOption> for CatalogOptionView {
    fn from(o: &CatalogOption) -> Self {
        let (kind, choices, min, max) = match o.kind {
            OptionKind::Bool => (OptionKindView::Bool, &[][..], None, None),
            OptionKind::Text => (OptionKindView::Text, &[][..], None, None),
            OptionKind::Choice(choices) => (OptionKindView::Choice, choices, None, None),
            OptionKind::Number { min, max } => (OptionKindView::Number, &[][..], min, max),
        };
        Self {
            key: o.key.to_owned(),
            kind,
            group: o.group.into(),
            default: o.default.map(str::to_owned),
            secret: o.secret,
            choices: choices
                .iter()
                .map(|c| ChoiceView {
                    value: c.value.to_owned(),
                    label: c.label.into(),
                })
                .collect(),
            min,
            max,
            label: o.label.into(),
            description: o.description.into(),
        }
    }
}

/// Client revisions the catalog was generated from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CatalogSourceView {
    /// `rustdesk/rustdesk` commit.
    pub rustdesk: String,
    /// `rustdesk/hbb_common` commit.
    pub hbb_common: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct CatalogView {
    pub source: CatalogSourceView,
    /// In the client's key list order.
    pub options: Vec<CatalogOptionView>,
}

/// The known client options with their type, default, group and texts.
#[utoipa::path(
    get,
    path = "/strategy-catalog",
    tag = "strategies",
    operation_id = "get_strategy_catalog",
    security(("console_session" = ["admin"])),
    responses((status = 200, description = "The catalog", body = CatalogView))
)]
pub async fn get_catalog() -> Json<CatalogView> {
    let source = catalog::source();
    Json(CatalogView {
        source: CatalogSourceView {
            rustdesk: source.rustdesk.to_owned(),
            hbb_common: source.hbb_common.to_owned(),
        },
        options: catalog::options().iter().map(Into::into).collect(),
    })
}
