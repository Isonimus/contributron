//! Durable event store with deduplication (slice 0009, ADR-0010).
//!
//! The store takes [`PullRequestObservation`]s, derives their identity from the observed
//! content the way [ADR-0008] decided, and persists them in a local SQLite file keyed by
//! that identity. Re-observing the same state is a no-op the database enforces, rather than
//! a check the caller remembered to perform.
//!
//! Owner scoping (standing invariant 7) is checked on the row the caller submits, not on a
//! wrapper. A store opened for one user id refuses to insert a row whose attribution names a
//! different one, and names the mismatch in the returned error.
//!
//! [ADR-0008]: ../../../adr/0008-event-identity-is-a-digest-of-observed-content.md

use contributron_domain::actor::{Attribution, ExecutingAgent, UserId, WorkerId};
use contributron_domain::event::{CanonicalEncodingError, EventId, PullRequestObservation};
use contributron_domain::pull_request::ObservedPullRequest;
use rusqlite::{Connection, OptionalExtension, Row, params};
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// A store of pull request observations, scoped to one owner.
#[derive(Debug)]
pub struct EventStore {
    connection: Connection,
    owner: UserId,
}

/// What a write did.
///
/// `Fresh` means a new row was persisted; `Duplicate` means a row with the same identity was
/// already there and nothing changed. A duplicate is not an error (ADR-0008: re-observing is
/// not a new fact), which is why this type and not `Result<(), AlreadyExists>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// A new row was inserted.
    Fresh,
    /// A row with this identity was already present; nothing was changed.
    Duplicate,
}

/// Something the store could not do.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// A row's owner does not match the store's owner (invariant 7).
    #[error(
        "owner mismatch: store is scoped to {expected:?} but the submitted row names {actual:?}"
    )]
    OwnerMismatch {
        /// Who the store belongs to.
        expected: String,
        /// Who the row said it belonged to.
        actual: String,
    },
    /// A SQLite call failed.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// An observation could not be canonicalized for the identity.
    #[error("canonical encoding: {0}")]
    Canonical(#[from] CanonicalEncodingError),
    /// A row's stored payload could not be decoded.
    #[error("decoding a stored row: {0}")]
    Decode(#[from] serde_json::Error),
    /// Formatting the write-time timestamp failed.
    ///
    /// Not reachable in practice: `OffsetDateTime::now_utc().format(&Rfc3339)` cannot fail
    /// for a valid UTC time between years 0 and 9999, which covers every clock the operator
    /// will run under. The variant exists because the upstream API returns a `Result`, and
    /// the quality bar forbids `expect` outside tests and startup; a `?` through a named
    /// variant is the honest alternative.
    #[error("formatting observed_at: {0}")]
    Clock(#[from] time::error::Format),
    /// The store file's schema version does not match the build's.
    ///
    /// Opening a file written by a build with a different schema or identity encoding is
    /// refused rather than best-effort-read: ADR-0008 already notes that an encoding change
    /// invalidates every stored identity, so the correct response is to let the operator
    /// decide (migrate, delete, or use a different file) rather than to risk mixing rows
    /// that cannot be compared.
    #[error("schema version mismatch: file has {found}, this build expects {expected}")]
    SchemaVersionMismatch {
        /// The version this build writes and reads.
        expected: i32,
        /// What the file's `user_version` reported.
        found: i32,
    },
}

/// SQLite's extended result code for a primary key conflict. Matched by number rather than
/// by string so a future rusqlite release that reorders an error display cannot silently
/// break the dedup path.
///
/// Only the specific extended code is matched. The generic constraint code (19, bare
/// `SQLITE_CONSTRAINT`) is deliberately not matched: a future schema addition (a UNIQUE,
/// CHECK or NOT NULL constraint) must not be swallowed as "already recorded" by this
/// codepath. If a new deduplication constraint is ever introduced, its specific extended
/// code is added here with a comment naming the constraint.
const SQLITE_CONSTRAINT_PRIMARYKEY: i32 = 1555;

/// The schema version recorded in `PRAGMA user_version`. Bumped only when the on-disk shape
/// of a row or the identity encoding changes in a way that an older build would read as
/// corrupt. Opening a file whose version does not match is an error, not a silent attempt
/// at best-effort reading: ADR-0008 already notes that an encoding change invalidates every
/// stored identity, so the correct response is to refuse and let the operator decide.
const SCHEMA_USER_VERSION: i32 = 1;

impl EventStore {
    /// Opens a store at `path`, creating the schema if it does not exist, scoped to `owner`.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError::Sqlite`] if the file cannot be opened or the schema cannot be
    /// created.
    pub fn open(path: &Path, owner: UserId) -> Result<Self, StoreError> {
        let connection = Connection::open(path)?;
        Self::ensure_schema(&connection)?;
        Ok(Self { connection, owner })
    }

    fn ensure_schema(connection: &Connection) -> Result<(), StoreError> {
        // The primary key is `(owner_user, event_id)` rather than `event_id` alone:
        // ADR-0008 derives the identity from observed content only, so two owners observing
        // the same pull request state agree on the identity. A single-column PK would make
        // the second owner's write collide with the first owner's row and, since every read
        // path is scoped by `owner_user`, the second owner would see nothing while being
        // told "already recorded". That is silent data loss across the owner boundary; the
        // composite key makes dedup per-owner, which is what invariant 7 means in practice.
        let on_disk_version: i32 =
            connection.query_row("SELECT user_version FROM pragma_user_version", [], |row| {
                row.get(0)
            })?;

        if on_disk_version == 0 {
            connection.execute_batch(
                "CREATE TABLE IF NOT EXISTS events (
                    event_id    TEXT NOT NULL,
                    owner_user  TEXT NOT NULL,
                    worker      TEXT NOT NULL,
                    agent       TEXT NOT NULL,
                    observed_at TEXT NOT NULL,
                    payload     TEXT NOT NULL,
                    PRIMARY KEY (owner_user, event_id)
                );
                PRAGMA user_version = 1;",
            )?;
        } else if on_disk_version != SCHEMA_USER_VERSION {
            return Err(StoreError::SchemaVersionMismatch {
                expected: SCHEMA_USER_VERSION,
                found: on_disk_version,
            });
        }
        Ok(())
    }

    /// Who this store is scoped to.
    #[must_use]
    pub fn owner(&self) -> &UserId {
        &self.owner
    }

    /// Writes `observation` to the store.
    ///
    /// Owner scoping runs before the write: if the observation's attribution names a user
    /// other than this store's owner, nothing is persisted and [`StoreError::OwnerMismatch`]
    /// is returned. The check is on the submitted row (invariant 7), so a caller that tries
    /// to sidestep a wrapper still hits it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on an owner mismatch, a canonical encoding failure, or a
    /// SQLite error other than a primary key conflict (which is mapped to
    /// [`WriteOutcome::Duplicate`]).
    pub fn write(&self, observation: &PullRequestObservation) -> Result<WriteOutcome, StoreError> {
        if observation.attribution.user.as_str() != self.owner.as_str() {
            return Err(StoreError::OwnerMismatch {
                expected: self.owner.as_str().to_owned(),
                actual: observation.attribution.user.as_str().to_owned(),
            });
        }

        let event_id = observation.event_id()?;
        let payload = serde_json::to_string(&observation.pull_request)?;
        let agent = serde_json::to_string(&observation.attribution.agent)?;
        let observed_at = OffsetDateTime::now_utc().format(&Rfc3339)?;

        let result = self.connection.execute(
            "INSERT INTO events (event_id, owner_user, worker, agent, observed_at, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                event_id.as_str(),
                observation.attribution.user.as_str(),
                observation.attribution.worker.as_str(),
                agent,
                observed_at,
                payload,
            ],
        );

        match result {
            Ok(_) => Ok(WriteOutcome::Fresh),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.extended_code == SQLITE_CONSTRAINT_PRIMARYKEY =>
            {
                // The row for this identity and owner is already there. The first writer's
                // attribution and observed_at remain; this is "first-write-wins", the
                // contract the dedup scenario asserts.
                Ok(WriteOutcome::Duplicate)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Reads the observation with `event_id`, if there is one.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on a SQLite error or a stored row the current code cannot
    /// decode.
    pub fn read(&self, event_id: &EventId) -> Result<Option<PullRequestObservation>, StoreError> {
        let maybe = self
            .connection
            .query_row(
                "SELECT owner_user, worker, agent, payload
                 FROM events
                 WHERE event_id = ?1 AND owner_user = ?2",
                params![event_id.as_str(), self.owner.as_str()],
                extract_strings,
            )
            .optional()?;
        maybe.map(decode_from_strings).transpose()
    }

    /// All observations in this store, ordered by event identity.
    ///
    /// The order is deterministic, so a render over the result is the same bytes on every
    /// call. Owner-scoped in the query clause rather than filtered in Rust: a store opened
    /// for one owner cannot read rows another process wrote under a different owner, which
    /// matters once the team model lands.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on a SQLite error or a stored row the current code cannot
    /// decode.
    pub fn list(&self) -> Result<Vec<PullRequestObservation>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT owner_user, worker, agent, payload
             FROM events
             WHERE owner_user = ?1
             ORDER BY event_id ASC",
        )?;
        let rows: Vec<StoredColumns> = statement
            .query_map(params![self.owner.as_str()], extract_strings)?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(decode_from_strings).collect()
    }
}

struct StoredColumns {
    owner: String,
    worker: String,
    agent_json: String,
    payload_json: String,
}

fn extract_strings(row: &Row<'_>) -> rusqlite::Result<StoredColumns> {
    Ok(StoredColumns {
        owner: row.get(0)?,
        worker: row.get(1)?,
        agent_json: row.get(2)?,
        payload_json: row.get(3)?,
    })
}

fn decode_from_strings(columns: StoredColumns) -> Result<PullRequestObservation, StoreError> {
    let pull_request: ObservedPullRequest = serde_json::from_str(&columns.payload_json)?;
    let agent: ExecutingAgent = serde_json::from_str(&columns.agent_json)?;
    let attribution = Attribution {
        user: UserId::new(columns.owner),
        worker: WorkerId::new(columns.worker),
        agent,
    };
    Ok(PullRequestObservation::new(attribution, pull_request))
}
