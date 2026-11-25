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
use sha2::{Digest, Sha256};
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
    /// A [`ReactionRecord`] carried a `NoAgent` attribution. Invariant 6 requires every
    /// recorded operation to name all three identities; a reaction with no agent cannot
    /// be stored.
    #[error("reaction record has no agent id (invariant 6 requires one)")]
    MissingAgentId,
    /// A stored reaction row's `outcome` column held a value this build does not know.
    /// The store refuses the decode rather than silently mapping it to `AlreadySettled`.
    #[error("unknown reaction outcome in stored row: {value:?}")]
    UnknownReactionOutcome {
        /// The unrecognised column value.
        value: String,
    },
    /// A stored reaction row's `pr_number` column held a negative value.
    #[error("stored reaction has negative pr_number: {value}")]
    NegativePrNumber {
        /// The unrecognised column value.
        value: i64,
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

const EVENTS_DDL: &str = "CREATE TABLE IF NOT EXISTS events (
    event_id    TEXT NOT NULL,
    owner_user  TEXT NOT NULL,
    worker      TEXT NOT NULL,
    agent       TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    payload     TEXT NOT NULL,
    PRIMARY KEY (owner_user, event_id)
);";

/// Reaction records persisted per rule fire (slice 0019). One row per receipt: a tick where
/// two rules both reported `Posted` writes two rows, one per `agent_id`. The reaction
/// identity derives from `(event_id, agent_id)` so a retry (which the react layer makes
/// idempotent) finds its earlier row as `WriteOutcome::Duplicate` and invariant 6 holds
/// without a second record.
const REACTIONS_DDL: &str = "CREATE TABLE IF NOT EXISTS reactions (
    reaction_id TEXT NOT NULL,
    owner_user  TEXT NOT NULL,
    worker      TEXT NOT NULL,
    agent_id    TEXT NOT NULL,
    pr_owner    TEXT NOT NULL,
    pr_repo     TEXT NOT NULL,
    pr_number   INTEGER NOT NULL,
    outcome     TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (owner_user, reaction_id)
);";

/// The schema version recorded in `PRAGMA user_version`. Bumped only when the on-disk shape
/// of a row or the identity encoding changes in a way that an older build would read as
/// corrupt. Opening a file whose version does not match is an error, not a silent attempt
/// at best-effort reading: ADR-0008 already notes that an encoding change invalidates every
/// stored identity, so the correct response is to refuse and let the operator decide.
const SCHEMA_USER_VERSION: i32 = 3;

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

        match on_disk_version {
            0 => {
                // Fresh file: create at the current version. The PRAGMA is kept inside the
                // same execute_batch so a crash between the DDL and the version write cannot
                // leave a table whose shape is ahead of its advertised version.
                connection.execute_batch(&format!(
                    "{EVENTS_DDL}
                     {REACTIONS_DDL}
                     PRAGMA user_version = {SCHEMA_USER_VERSION};",
                ))?;
            }
            2 => {
                // Slice 0019 added the `reactions` table; migration from v2 is additive and
                // leaves `events` rows untouched (DoD scenario 5).
                connection.execute_batch(&format!(
                    "{REACTIONS_DDL}
                     PRAGMA user_version = {SCHEMA_USER_VERSION};",
                ))?;
            }
            v if v == SCHEMA_USER_VERSION => {}
            v => {
                return Err(StoreError::SchemaVersionMismatch {
                    expected: SCHEMA_USER_VERSION,
                    found: v,
                });
            }
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

    /// Writes one reaction row (slice 0019).
    ///
    /// Owner scoping runs the same way as [`write`]: a row whose attribution names an owner
    /// other than this store's owner is refused with [`StoreError::OwnerMismatch`]. Reaction
    /// identity is derived from `(event_id, agent_id)`, so a retry of the same rule fire on
    /// the same observation returns [`WriteOutcome::Duplicate`] without a second row. One
    /// row per rule fire is invariant 6's atom: each rule is an operation accountable to its
    /// three identities.
    ///
    /// [`write`]: Self::write
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on an owner mismatch, a timestamp-format error, or a SQLite
    /// error other than a primary key conflict.
    pub fn write_reaction(&self, row: &ReactionRecord) -> Result<WriteOutcome, StoreError> {
        if row.attribution.user.as_str() != self.owner.as_str() {
            return Err(StoreError::OwnerMismatch {
                expected: self.owner.as_str().to_owned(),
                actual: row.attribution.user.as_str().to_owned(),
            });
        }
        let agent_id = match &row.attribution.agent {
            ExecutingAgent::Agent { id } => id.as_str().to_owned(),
            ExecutingAgent::NoAgent => {
                return Err(StoreError::MissingAgentId);
            }
        };
        let reaction_id = reaction_id(&row.event_id, &agent_id);
        let observed_at = OffsetDateTime::now_utc().format(&Rfc3339)?;
        let pr_number = i64::try_from(row.pr_number).unwrap_or(i64::MAX);
        let result = self.connection.execute(
            "INSERT INTO reactions (
                reaction_id, owner_user, worker, agent_id,
                pr_owner, pr_repo, pr_number, outcome, observed_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                reaction_id,
                row.attribution.user.as_str(),
                row.attribution.worker.as_str(),
                agent_id,
                row.pr_owner,
                row.pr_repo,
                pr_number,
                row.outcome.as_str(),
                observed_at,
            ],
        );
        match result {
            Ok(_) => Ok(WriteOutcome::Fresh),
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.extended_code == SQLITE_CONSTRAINT_PRIMARYKEY =>
            {
                Ok(WriteOutcome::Duplicate)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// All reaction rows in this store, in reaction-id order.
    ///
    /// Deterministic order so a test can assert against the full set without sorting.
    /// Owner-scoped in the query clause the same way [`list`] is.
    ///
    /// [`list`]: Self::list
    ///
    /// # Errors
    ///
    /// Returns [`StoreError`] on a SQLite error or a row the current code cannot decode.
    pub fn list_reactions(&self) -> Result<Vec<StoredReaction>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT reaction_id, owner_user, worker, agent_id,
                    pr_owner, pr_repo, pr_number, outcome, observed_at
             FROM reactions
             WHERE owner_user = ?1
             ORDER BY reaction_id ASC",
        )?;
        let rows = statement
            .query_map(params![self.owner.as_str()], |row| {
                Ok(StoredReactionColumns {
                    reaction_id: row.get(0)?,
                    owner: row.get(1)?,
                    worker: row.get(2)?,
                    agent_id: row.get(3)?,
                    pr_owner: row.get(4)?,
                    pr_repo: row.get(5)?,
                    pr_number: row.get(6)?,
                    outcome: row.get(7)?,
                    observed_at: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter().map(decode_reaction).collect()
    }
}

/// One rule-fire the poll loop submits for persistence (slice 0019).
///
/// Built by the poll loop out of the reactor's attribution and the tick's observation
/// identity. Not derived from a `ReactionReceipt` directly: the react crate's receipt type
/// carries kind-specific payload the slice 0019 schema deliberately omits, so the carrier
/// here is narrower.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionRecord {
    /// The three identities accountable for this reaction. The `agent` field must be an
    /// [`ExecutingAgent::Agent`]; a `NoAgent` attribution returns [`StoreError::MissingAgentId`].
    pub attribution: Attribution,
    /// The pull request's owner (e.g. "octo-org").
    pub pr_owner: String,
    /// The pull request's repo (e.g. "contributron").
    pub pr_repo: String,
    /// The pull request's number.
    pub pr_number: u64,
    /// What the rule did.
    pub outcome: ReactionRecordOutcome,
    /// The identity of the observation this reaction fired against. Pairs with `agent_id`
    /// to form the reaction's own identity.
    pub event_id: EventId,
}

/// What a persisted reaction was (slice 0019).
///
/// Only `Posted` and `AlreadySettled` are persisted: `NotApplicable` has no operation to
/// record, and `Failed` never completed an operation against the provider (the error
/// surfaces on the tick stream instead). The variants here mirror the two success cases of
/// [`contributron_poll::ReactionReport`] rather than re-exporting that type so the store
/// crate keeps its dependency direction (store → domain, never store → poll).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionRecordOutcome {
    /// The rule performed a write against the provider.
    Posted,
    /// The rule confirmed the desired state was already in place; no write.
    AlreadySettled,
}

impl ReactionRecordOutcome {
    /// Column text written to the `outcome` column, and parsed back by `list_reactions`.
    fn as_str(self) -> &'static str {
        match self {
            Self::Posted => "posted",
            Self::AlreadySettled => "already-settled",
        }
    }

    fn parse(value: &str) -> Result<Self, StoreError> {
        match value {
            "posted" => Ok(Self::Posted),
            "already-settled" => Ok(Self::AlreadySettled),
            other => Err(StoreError::UnknownReactionOutcome {
                value: other.to_owned(),
            }),
        }
    }
}

/// A reaction row read back from the store (slice 0019 round-trip surface).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredReaction {
    /// The derived reaction identity.
    pub reaction_id: String,
    /// The three identities, reconstructed from the stored columns.
    pub attribution: Attribution,
    /// The pull request's owner.
    pub pr_owner: String,
    /// The pull request's repo.
    pub pr_repo: String,
    /// The pull request's number.
    pub pr_number: u64,
    /// What the rule did.
    pub outcome: ReactionRecordOutcome,
    /// The timestamp the row was written with.
    pub observed_at: String,
}

struct StoredReactionColumns {
    reaction_id: String,
    owner: String,
    worker: String,
    agent_id: String,
    pr_owner: String,
    pr_repo: String,
    pr_number: i64,
    outcome: String,
    observed_at: String,
}

fn decode_reaction(columns: StoredReactionColumns) -> Result<StoredReaction, StoreError> {
    let pr_number = u64::try_from(columns.pr_number).map_err(|_| StoreError::NegativePrNumber {
        value: columns.pr_number,
    })?;
    let attribution = Attribution {
        user: UserId::new(columns.owner),
        worker: WorkerId::new(columns.worker),
        agent: ExecutingAgent::Agent {
            id: contributron_domain::actor::AgentId::new(columns.agent_id),
        },
    };
    Ok(StoredReaction {
        reaction_id: columns.reaction_id,
        attribution,
        pr_owner: columns.pr_owner,
        pr_repo: columns.pr_repo,
        pr_number,
        outcome: ReactionRecordOutcome::parse(&columns.outcome)?,
        observed_at: columns.observed_at,
    })
}

/// Derives the reaction identity from `(event_id, agent_id)`. SHA-256 over
/// `"{event_id}:{agent_id}"` — same hasher shape as [`EventId`] in the domain crate. The
/// colon separator cannot appear inside an `event_id` (lowercase hex only) and the
/// react crate's agent id literals are `rule:*` strings whose own colon is part of the
/// identity namespace, so there is no ambiguity.
fn reaction_id(event_id: &EventId, agent_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_id.as_str().as_bytes());
    hasher.update(b":");
    hasher.update(agent_id.as_bytes());
    hex(&hasher.finalize())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: [char; 16] = [
        '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
    ];
    let mut out = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let (high, low) = (usize::from(byte / 16), usize::from(byte % 16));
        out.push(DIGITS[high]);
        out.push(DIGITS[low]);
    }
    out
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
