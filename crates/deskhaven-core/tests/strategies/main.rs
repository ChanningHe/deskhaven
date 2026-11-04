//! Strategy service (M2-13): CRUD, assignments, effective strategy, cache invalidation,
//! `modified_at`. Every test runs on SQLite and PostgreSQL.

mod assignments;
mod console;
mod crud;
mod deletion;
mod effective;
mod group_hooks;
mod invalidation;
mod modified_at;
mod support;
