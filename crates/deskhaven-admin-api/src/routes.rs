//! Route assembly. [`AdminRoutes`] is the only way to add an admin API route: it takes
//! the output of `utoipa_axum::routes!` (so every handler has a `#[utoipa::path]`),
//! reads each operation's [`Permission`] from it (none -> [`RouteError`], and the server
//! does not start), puts the handler behind the gate for that permission, and documents
//! the gate's own error responses in the OpenAPI operation.

use std::sync::Arc;

use axum::Router;
use axum::http::Method;
use axum::middleware::from_fn;
use axum::routing::MethodRouter;
use deskhaven_core::state::AppState;
use utoipa::openapi::path::{Operation, PathItem, Paths};
use utoipa::openapi::schema::Schema;
use utoipa::openapi::{Content, OpenApi, Ref, RefOr, ResponseBuilder};
use utoipa_axum::router::UtoipaMethodRouter;

use crate::gate::{RouteGuard, gate};
use crate::openapi;
use crate::origin::TrustedOrigins;
use crate::permission::Permission;

/// Prefix of every admin API route (08 §6).
pub const ADMIN_API_V1: &str = "/admin/api/v1";

type Schemas = Vec<(String, RefOr<Schema>)>;

/// An operation that cannot be served as declared.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("admin route {method} {path}: {reason}")]
pub struct RouteError {
    pub method: String,
    pub path: String,
    pub reason: &'static str,
}

/// Admin API routes under construction (see the module docs).
#[derive(Default)]
pub struct AdminRoutes {
    entries: Vec<Entry>,
    origins: TrustedOrigins,
}

struct Entry {
    /// Absolute path, in OpenAPI (= axum 0.8) syntax.
    path: String,
    item: PathItem,
    schemas: Schemas,
    methods: MethodRouter<Arc<AppState>>,
    guard: RouteGuard,
}

impl AdminRoutes {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds the operations of one `routes!(...)` call: one path, one permission (put
    /// handlers with different permissions in separate calls).
    pub fn routes(self, routes: UtoipaMethodRouter<Arc<AppState>>) -> Result<Self, RouteError> {
        let (schemas, paths, methods) = routes;
        let (relative, mut item) = single_path(paths)?;
        let path = absolute(&relative);
        let permission = route_permission(&path, &item)?;
        self.reject_duplicates(&path, &item)?;
        for (method, operation) in operations_mut(&mut item) {
            openapi::add_gate_docs(operation, &method, permission);
        }
        let operations = operations_mut(&mut item)
            .map(|(method, op)| (method, op.operation_id.clone().unwrap_or_default()))
            .collect();
        let entry = Entry {
            path,
            item,
            schemas,
            methods,
            guard: RouteGuard {
                permission,
                operations,
                origins: TrustedOrigins::default(),
            },
        };
        let mut entries = self.entries;
        entries.push(entry);
        Ok(Self { entries, ..self })
    }

    /// Adds every route of `other`; the same operation twice is a [`RouteError`].
    pub fn merge(self, other: Self) -> Result<Self, RouteError> {
        for entry in &other.entries {
            self.reject_duplicates(&entry.path, &entry.item)?;
        }
        let mut entries = self.entries;
        entries.extend(other.entries);
        Ok(Self { entries, ..self })
    }

    /// axum panics (at router construction, after the listeners are bound) on a second
    /// handler for one method and path, and on one path shape with differently named
    /// parameters (`/users/{id}` and `/users/{user_id}`); fail cleanly instead.
    fn reject_duplicates(&self, path: &str, item: &PathItem) -> Result<(), RouteError> {
        let shape = path_shape(path);
        for entry in self.entries.iter().filter(|e| path_shape(&e.path) == shape) {
            let error = |method: &Method, reason| RouteError {
                method: method.to_string(),
                path: path.to_owned(),
                reason,
            };
            if entry.path != path {
                return Err(error(&Method::GET, "path parameters named differently"));
            }
            let taken = operations(&entry.item).map(|(method, _)| method);
            for method in taken {
                if operations(item).any(|(m, _)| m == method) {
                    return Err(error(&method, "operation declared twice"));
                }
            }
        }
        Ok(())
    }

    /// The OpenAPI document of these routes.
    pub fn openapi(&self) -> OpenApi {
        let mut doc = openapi::base();
        for entry in &self.entries {
            let paths = &mut doc.paths.paths;
            match paths.get_mut(&entry.path) {
                Some(item) => item.merge_operations(entry.item.clone()),
                None => {
                    paths.insert(entry.path.clone(), entry.item.clone());
                }
            }
            let components = doc.components.get_or_insert_with(Default::default);
            components.schemas.extend(entry.schemas.iter().cloned());
        }
        doc
    }

    /// Origins the console is served from (`public_url`, `admin_public_url`), accepted by
    /// the same-origin check besides the request's own `Host`.
    #[must_use]
    pub fn trusted_origins(self, origins: TrustedOrigins) -> Self {
        Self { origins, ..self }
    }

    /// The routes, each behind its gate, ready to merge into the admin listener's router.
    pub fn into_router(self, state: Arc<AppState>) -> Router {
        let origins = self.origins;
        let router = self
            .entries
            .into_iter()
            .fold(Router::new(), |router, entry| {
                let state = Arc::clone(&state);
                let guard = RouteGuard {
                    origins: origins.clone(),
                    ..entry.guard
                };
                let gated = entry.methods.layer(from_fn(move |req, next| {
                    gate(Arc::clone(&state), guard.clone(), req, next)
                }));
                router.route(&entry.path, gated)
            });
        router.with_state(state)
    }
}

fn single_path(paths: Paths) -> Result<(String, PathItem), RouteError> {
    let mut entries = paths.paths.into_iter();
    match (entries.next(), entries.next()) {
        (Some(only), None) => Ok(only),
        (first, _) => Err(RouteError {
            method: "*".to_owned(),
            path: first.map(|(path, _)| path).unwrap_or_default(),
            reason: "one routes!(...) call must serve exactly one path",
        }),
    }
}

/// `path` with parameter names erased: `{id}` -> `{}`, `{*rest}` -> `{*}`.
fn path_shape(path: &str) -> String {
    path.split('/')
        .map(
            |segment| match segment.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                Some(name) if name.starts_with('*') => "{*}",
                Some(_) => "{}",
                None => segment,
            },
        )
        .collect::<Vec<_>>()
        .join("/")
}

fn absolute(relative: &str) -> String {
    match relative {
        "" | "/" => ADMIN_API_V1.to_owned(),
        path => format!("{ADMIN_API_V1}{path}"),
    }
}

/// The one permission all operations of `item` declare.
fn route_permission(path: &str, item: &PathItem) -> Result<Permission, RouteError> {
    let error = |method: &Method, reason| RouteError {
        method: method.to_string(),
        path: path.to_owned(),
        reason,
    };
    let mut found = None;
    for (method, operation) in operations(item) {
        let permission = Permission::of_operation(operation).map_err(|r| error(&method, r))?;
        match found {
            Some(other) if other != permission => {
                return Err(error(
                    &method,
                    "permissions differ within one routes!(...) call",
                ));
            }
            _ => found = Some(permission),
        }
    }
    found.ok_or_else(|| error(&Method::GET, "no operation"))
}

fn operations(item: &PathItem) -> impl Iterator<Item = (Method, &Operation)> {
    [
        (Method::GET, &item.get),
        (Method::PUT, &item.put),
        (Method::POST, &item.post),
        (Method::DELETE, &item.delete),
        (Method::OPTIONS, &item.options),
        (Method::HEAD, &item.head),
        (Method::PATCH, &item.patch),
        (Method::TRACE, &item.trace),
    ]
    .into_iter()
    .filter_map(|(method, op)| op.as_ref().map(|op| (method, op)))
}

fn operations_mut(item: &mut PathItem) -> impl Iterator<Item = (Method, &mut Operation)> {
    [
        (Method::GET, &mut item.get),
        (Method::PUT, &mut item.put),
        (Method::POST, &mut item.post),
        (Method::DELETE, &mut item.delete),
        (Method::OPTIONS, &mut item.options),
        (Method::HEAD, &mut item.head),
        (Method::PATCH, &mut item.patch),
        (Method::TRACE, &mut item.trace),
    ]
    .into_iter()
    .filter_map(|(method, op)| op.as_mut().map(|op| (method, op)))
}

/// A JSON response whose body is the `ErrorEnvelope` schema.
pub(crate) fn error_response(description: &str) -> RefOr<utoipa::openapi::Response> {
    let content = Content::new(Some(Ref::from_schema_name(openapi::ERROR_SCHEMA)));
    ResponseBuilder::new()
        .description(description)
        .content("application/json", content)
        .build()
        .into()
}
