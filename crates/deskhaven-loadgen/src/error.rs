use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::time::Duration;

#[derive(Debug, thiserror::Error)]
pub enum LoadgenError {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("cannot {action} {}", path.display())]
    Io {
        action: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("cannot start the server {}", path.display())]
    Spawn {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("the server exited before it was ready ({status}); see {}", log.display())]
    ServerExited { status: ExitStatus, log: PathBuf },
    #[error("the server was not ready within {limit:?}; see {}", log.display())]
    NotReady { limit: Duration, log: PathBuf },
    #[error("cannot stop the server (pid {pid})")]
    Stop {
        pid: u32,
        #[source]
        source: io::Error,
    },
    #[error("cannot read /proc/{pid}/{file}")]
    Proc {
        pid: u32,
        file: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("cannot open the database for the write counters")]
    DbConnect(#[from] deskhaven_db::DbError),
    #[error("database write counters failed")]
    Db(#[from] sea_orm::DbErr),
    #[error("cannot start the async runtime")]
    Runtime(#[source] io::Error),
    #[error("interrupted by {0}; the server was stopped")]
    Interrupted(&'static str),
}
