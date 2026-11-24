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
const SCHEMA_USER_VERSION: i32 = 2;

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
            // Fresh file: create at the current version. The PRAGMA is kept inside the
            // same execute_batch so a crash between the DDL and the version write cannot
            // leave a table whose shape is ahead of its advertised version.
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
                PRAGMA user_version = 2;",
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

    /// The most recent observation for the given pull-request number, by wall-clock write
    /// time, or `None` if no row exists for that number in this owner's scope.
    ///
    /// The polling loop uses this at task startup to seed its in-memory `prior` from the
    /// store, so a reactor can detect a transition whose two halves were observed in
    /// different process lifetimes (slice 0014 scenario 1, review finding from
    /// 2026-11-24). Filtering is on `json_extract(payload, '$.number')` because
    /// [`ObservedPullRequest`] carries the number in its JSON payload (serde's default for a
    /// newtype tuple struct is the bare inner value). There is no index on the extracted
    /// field: a watch tracks a handful of pull requests per operator and this runs once per
    /// target at startup, so a scan is fine. The multi-repository-same-number corner
    /// (`foo/bar#7` and `baz/qux#7` in the same store) returns whichever row was written
    /// last regardless of repository; ledgered, since `ObservedPullRequest` does not yet
    /// carry owner/repo.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on a SQLite error or a stored row the current code cannot
    /// decode.
    pub fn latest_for_number(
        &self,
        number: u64,
    ) -> Result<Option<ObservedPullRequest>, StoreError> {
        // SQLite integers are i64; a `PullRequestNumber` is non-zero but has no upper bound,
        // so a value past `i64::MAX` saturates to `i64::MAX` and matches nothing, which
        // yields `None` without crashing the query path.
        let number_i64 = i64::try_from(number).unwrap_or(i64::MAX);
        let row: Option<String> = self
            .connection
            .query_row(
                "SELECT payload
                 FROM events
                 WHERE owner_user = ?1
                   AND json_extract(payload, '$.number') = ?2
                 ORDER BY observed_at DESC
                 LIMIT 1",
                params![self.owner.as_str(), number_i64],
                |r| r.get(0),
            )
            .optional()?;
        match row {
            Some(payload) => Ok(Some(serde_json::from_str(&payload)?)),
            None => Ok(None),
        }
    }

    /// The most recent observation of a pull request number whose mergeable field is one
    /// of the settled values (`MERGEABLE` or `CONFLICTING`), skipping `UNKNOWN` rows.
    ///
    /// Slice 0015's merge-conflicts-cleared rule uses this to look past the immediate
    /// in-memory prior: GitHub returns `mergeable: null` (→ Unknown) for seconds-to-
    /// minutes after a push, and a single Unknown tick erases the preceding Conflicting
    /// evidence from the loop's `prior` cache (finding 1 of the 2026-11-25 review). The
    /// store still carries the earlier settled row, and this query returns it.
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on a SQLite error or a row the current code cannot decode.
    pub fn latest_settled_mergeable_for_number(
        &self,
        number: u64,
    ) -> Result<Option<ObservedPullRequest>, StoreError> {
        let number_i64 = i64::try_from(number).unwrap_or(i64::MAX);
        let row: Option<String> = self
            .connection
            .query_row(
                "SELECT payload
                 FROM events
                 WHERE owner_user = ?1
                   AND json_extract(payload, '$.number') = ?2
                   AND json_extract(payload, '$.mergeable') IN ('MERGEABLE', 'CONFLICTING')
                 ORDER BY observed_at DESC
                 LIMIT 1",
                params![self.owner.as_str(), number_i64],
                |r| r.get(0),
            )
            .optional()?;
        match row {
            Some(payload) => Ok(Some(serde_json::from_str(&payload)?)),
            None => Ok(None),
        }
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
