//! A database error with a chosen SQLSTATE, to test how callers classify driver errors.

use std::borrow::Cow;
use std::error::Error;
use std::fmt;

use sea_orm::sqlx;
use sea_orm::sqlx::error::{DatabaseError, ErrorKind};
use sea_orm::{DbErr, RuntimeErr};

#[derive(Debug)]
struct Coded(&'static str);

impl fmt::Display for Coded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database error {}", self.0)
    }
}

impl Error for Coded {}

impl DatabaseError for Coded {
    fn message(&self) -> &str {
        "coded"
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0))
    }

    fn as_error(&self) -> &(dyn Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> ErrorKind {
        ErrorKind::Other
    }
}

pub fn coded_error(code: &'static str) -> DbErr {
    let error = sqlx::Error::Database(Box::new(Coded(code)));
    DbErr::Exec(RuntimeErr::SqlxError(error.into()))
}
