//! Replayable outbox processing and disposable Tantivy index generations.
//!
//! Storage access is asynchronous through the backend-neutral contract. Sequence-sensitive
//! sections are serialized by an async writer gate, so the guard can be held across the storage
//! awaits without making the futures non-Send.

use crate::ApplicationError;
use sandpost_core::{MessageIdentifier, MessageSequence};
use sandpost_search::{IndexedDocument, MessageQuery, SearchIndex};
use sandpost_storage::{SearchSynchronization, Storage};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use tokio::sync::watch;
use uuid::Uuid;

const INDEX_BATCH_SIZE: usize = 64;
const MAXIMUM_COALESCED_OPERATION_COUNT: usize = 1024;
const MAXIMUM_TEMPORARY_REBUILD_ATTEMPTS: usize = 3;

/// One installation's search writer and atomically replaceable reader generation.
#[derive(Clone)]
pub struct SearchCoordinator(Arc<SearchState>);

struct SearchState {
    storage: Arc<dyn Storage>,
    directory: Option<PathBuf>,
    current: RwLock<Arc<SearchIndex>>,
    writer_gate: tokio::sync::Mutex<()>,
    authoritative_writer: bool,
    observed_sequence: AtomicU64,
    committed_sequence: watch::Sender<u64>,
    closed: AtomicBool,
    retired: Mutex<Vec<(PathBuf, Weak<SearchIndex>)>>,
    // Holding this file retains the process-exclusive installation search lock.
    installation_lock: Mutex<Option<File>>,
}

impl SearchCoordinator {
    /// Build an isolated derived index for embedded tests and storage-only callers.
    pub async fn memory(storage: Arc<dyn Storage>) -> Result<Self, ApplicationError> {
        let coordinator = Self(Arc::new(SearchState {
            storage,
            directory: None,
            current: RwLock::new(Arc::new(SearchIndex::memory()?)),
            writer_gate: tokio::sync::Mutex::new(()),
            authoritative_writer: false,
            observed_sequence: AtomicU64::new(0),
            committed_sequence: watch::channel(0).0,
            closed: AtomicBool::new(false),
            retired: Mutex::new(Vec::new()),
            installation_lock: Mutex::new(None),
        }));
        coordinator.rebuild().await?;
        Ok(coordinator)
    }

    /// Open a durable generation, replay pending operations, or rebuild damaged state.
    /// A second process must not acknowledge operations against a different reader.
    pub async fn open(
        storage: Arc<dyn Storage>,
        directory: PathBuf,
    ) -> Result<Self, ApplicationError> {
        fs::create_dir_all(&directory)?;
        let installation_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join("writer.lock"))?;
        installation_lock.try_lock().map_err(|error| {
            ApplicationError::Unavailable(format!("search writer is already running: {error}"))
        })?;
        let status = storage.search_status().await?;
        let existing = read_generation(&directory).and_then(|generation| {
            let index = SearchIndex::open(directory.join(generation))?;
            if index.committed_sequence()? < status.indexed_sequence
                || index.committed_sequence()? > status.latest_sequence
            {
                return Err(ApplicationError::Unavailable(
                    "index commit is behind the authoritative SQL watermark".into(),
                ));
            }
            Ok(index)
        });
        let (index, needs_rebuild) = match existing {
            Ok(index) => {
                tracing::info!(indexed_sequence = status.indexed_sequence, "Tantivy opened");
                (index, false)
            }
            Err(error) => {
                tracing::warn!(%error, "search generation unavailable; rebuilding from SQL");
                (SearchIndex::memory()?, true)
            }
        };
        let coordinator = Self(Arc::new(SearchState {
            storage,
            directory: Some(directory),
            current: RwLock::new(Arc::new(index)),
            writer_gate: tokio::sync::Mutex::new(()),
            authoritative_writer: true,
            observed_sequence: AtomicU64::new(status.indexed_sequence),
            committed_sequence: watch::channel(status.indexed_sequence).0,
            closed: AtomicBool::new(false),
            retired: Mutex::new(Vec::new()),
            installation_lock: Mutex::new(Some(installation_lock)),
        }));
        if needs_rebuild {
            coordinator.rebuild().await?;
        } else {
            coordinator.catch_up().await?;
        }
        coordinator.clean_startup_generations()?;
        Ok(coordinator)
    }

    /// Execute an already authorized structured query against one reader generation.
    pub fn search(&self, query: &MessageQuery) -> Result<Vec<MessageIdentifier>, ApplicationError> {
        let current = self.current()?;
        Ok(current.search(query)?)
    }

    /// Carry the authorized indexed revision into authoritative lightweight SQL hydration.
    pub fn search_hits(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<(MessageIdentifier, u64)>, ApplicationError> {
        Ok(self.current()?.search_hits(query)?)
    }

    /// Retain indexed sequence progress when authoritative SQL omits a search hit.
    pub fn search_hits_with_sequences(
        &self,
        query: &MessageQuery,
    ) -> Result<Vec<(MessageIdentifier, u64, MessageSequence)>, ApplicationError> {
        Ok(self.current()?.search_hits_with_sequences(query)?)
    }

    /// Await committed reader progress without polling; closing the writer wakes all waiters.
    pub async fn wait_for_sequence(&self, sequence: u64) -> Result<(), ApplicationError> {
        let mut committed = self.0.committed_sequence.subscribe();
        loop {
            let observed = *committed.borrow_and_update();
            self.require_writer_open()?;
            if observed >= sequence {
                return Ok(());
            }
            committed.changed().await.map_err(|_| {
                ApplicationError::Unavailable("search progress channel closed".into())
            })?;
        }
    }

    /// Release installation and Tantivy writer ownership while preserving existing readers.
    /// Subsequent synchronization and rebuild operations fail rather than reacquiring ownership.
    pub async fn close_writer(&self) -> Result<(), ApplicationError> {
        let _gate = self.0.writer_gate.lock().await;
        if self.0.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let result = self.current().and_then(|index| {
            index.close_writer()?;
            Ok(())
        });
        let release = self
            .0
            .installation_lock
            .lock()
            .map(|mut installation_lock| {
                installation_lock.take();
            })
            .map_err(|_| poisoned_search());
        self.0
            .committed_sequence
            .send_replace(self.0.observed_sequence.load(Ordering::Acquire));
        result.and(release)
    }

    /// Refuse writes and waits after explicit lifecycle closure.
    fn require_writer_open(&self) -> Result<(), ApplicationError> {
        if self.0.closed.load(Ordering::Acquire) {
            Err(ApplicationError::Unavailable(
                "search writer is closed".into(),
            ))
        } else {
            Ok(())
        }
    }

    /// Report SQL's durable mutation and acknowledgement watermarks.
    pub async fn status(&self) -> Result<SearchSynchronization, ApplicationError> {
        let mut status = self.0.storage.search_status().await?;
        if !self.0.authoritative_writer {
            status.indexed_sequence = self.0.observed_sequence.load(Ordering::Acquire);
            status.pending_operations = status
                .latest_sequence
                .saturating_sub(status.indexed_sequence);
        }
        Ok(status)
    }

    /// Coalesce a bounded outbox prefix into at most 64 message replacements before committing.
    /// Replaying after either process or acknowledgement failure is idempotent.
    pub async fn synchronize_batch(&self) -> Result<usize, ApplicationError> {
        for attempt in 0..=MAXIMUM_TEMPORARY_REBUILD_ATTEMPTS {
            let gate = self.0.writer_gate.lock().await;
            self.require_writer_open()?;
            self.clean_retired_generations()?;
            let current = self.current()?;
            let after = self.0.observed_sequence.load(Ordering::Acquire);
            if let Some((count, watermark)) = self.apply_pending(&current, after).await? {
                if count != 0 {
                    self.acknowledge(watermark).await?;
                    tracing::debug!(count, watermark, "search batch indexed");
                }
                return Ok(count);
            }
            drop(gate);
            if attempt == MAXIMUM_TEMPORARY_REBUILD_ATTEMPTS {
                break;
            }
            self.rebuild().await?;
        }
        Err(ApplicationError::Unavailable(
            "durable search acknowledgement keeps advancing during temporary synchronization"
                .into(),
        ))
    }

    /// Drain pending startup operations without admitting a permanently partial reader.
    pub async fn catch_up(&self) -> Result<(), ApplicationError> {
        let mut recovered = 0;
        while self.synchronize_batch().await? != 0 {
            recovered += 1;
        }
        if recovered != 0 {
            tracing::info!(batches = recovered, "pending search operations recovered");
        }
        Ok(())
    }

    /// Stream storage into a new generation while ingestion continues, then replay and publish.
    /// Durable writers prevent pruning; temporary writers retry when durable pruning advances.
    pub async fn rebuild(&self) -> Result<SearchSynchronization, ApplicationError> {
        self.rebuild_after_baseline(|| Ok(())).await
    }

    /// Invoke the publication seam after the baseline, allowing deterministic recovery tests.
    pub async fn rebuild_after_baseline(
        &self,
        after_baseline: impl FnOnce() -> Result<(), ApplicationError>,
    ) -> Result<SearchSynchronization, ApplicationError> {
        let _gate = self.0.writer_gate.lock().await;
        self.require_writer_open()?;
        tracing::info!("search rebuild started");
        let mut after_baseline = Some(after_baseline);
        for _ in 0..MAXIMUM_TEMPORARY_REBUILD_ATTEMPTS {
            if let Some(status) = self.rebuild_generation(&mut after_baseline).await? {
                return Ok(status);
            }
        }
        Err(ApplicationError::Unavailable(
            "durable search acknowledgement keeps advancing during temporary rebuild".into(),
        ))
    }

    /// Build one generation under the writer gate, refusing publication after concurrent pruning.
    /// The test seam runs once even when a temporary baseline must be retried.
    async fn rebuild_generation(
        &self,
        after_baseline: &mut Option<impl FnOnce() -> Result<(), ApplicationError>>,
    ) -> Result<Option<SearchSynchronization>, ApplicationError> {
        let generation = format!("generation-{}", Uuid::new_v4());
        let rebuilt = Arc::new(match &self.0.directory {
            Some(directory) => SearchIndex::open(directory.join(&generation))?,
            None => SearchIndex::memory()?,
        });
        let baseline = self.0.storage.search_status().await?;
        let upper_sequence = self.0.storage.max_message_sequence().await?;
        let mut cursor = MessageSequence(0);
        let mut messages_indexed = 0usize;
        loop {
            let messages = self
                .0
                .storage
                .index_messages(cursor, Some(upper_sequence), INDEX_BATCH_SIZE)
                .await?;
            if messages.is_empty() {
                break;
            }
            for message in messages {
                cursor = message.sequence;
                rebuilt.upsert(&indexed_document(message))?;
                messages_indexed += 1;
            }
            // Keep the writer's buffered work and memory bounded for million-message stores.
            rebuilt.commit(baseline.indexed_sequence)?;
            tracing::debug!(messages_indexed, "search rebuild progress");
        }
        // Replay all retained operations, including pre-existing deletes and post-scan ingests.
        // Capture a cutoff so continuous SMTP traffic cannot prevent publication indefinitely.
        if let Some(after_baseline) = after_baseline.take() {
            after_baseline()?;
        }
        let cutoff = self.0.storage.search_status().await?.latest_sequence;
        let mut watermark = 0;
        loop {
            let (status, operations) = self
                .0
                .storage
                .search_batch(watermark, INDEX_BATCH_SIZE)
                .await?;
            if !self.0.authoritative_writer && status.indexed_sequence > baseline.indexed_sequence {
                return Ok(None);
            }
            let operations: Vec<_> = operations
                .into_iter()
                .take_while(|operation| operation.sequence <= cutoff)
                .collect();
            if operations.is_empty() {
                break;
            }
            let identifiers: Vec<_> = operations
                .iter()
                .map(|operation| operation.message_identifier)
                .collect();
            for identifier in &identifiers {
                rebuilt.delete(*identifier)?;
            }
            for message in self.0.storage.indexed_messages(&identifiers).await? {
                rebuilt.upsert(&indexed_document(message))?;
            }
            watermark = operations.last().expect("nonempty batch").sequence;
            rebuilt.commit(watermark)?;
        }
        // Acknowledgement may advance while the last replay batch is being committed.
        if !self.0.authoritative_writer
            && self.0.storage.search_status().await?.indexed_sequence > baseline.indexed_sequence
        {
            return Ok(None);
        }
        rebuilt.commit(cutoff)?;
        let previous_generation = self
            .0
            .directory
            .as_ref()
            .and_then(|directory| read_generation(directory).ok());
        if let Some(directory) = &self.0.directory {
            publish_generation(directory, &generation)?;
        }
        let previous = {
            let mut current = self.0.current.write().map_err(|_| poisoned_search())?;
            std::mem::replace(&mut *current, rebuilt)
        };
        if let (Some(directory), Some(previous_generation)) =
            (&self.0.directory, previous_generation)
            && previous_generation != generation
        {
            self.0.retired.lock().map_err(|_| poisoned_search())?.push((
                directory.join(previous_generation),
                Arc::downgrade(&previous),
            ));
        }
        drop(previous);
        self.clean_retired_generations()?;
        // Publication precedes acknowledgement. Interrupted publication is always recoverable.
        self.acknowledge(cutoff).await?;
        tracing::info!(messages_indexed, cutoff, "search rebuild completed");
        Ok(Some(self.status().await?))
    }

    /// Only the process-owned durable index may acknowledge and prune storage operations.
    async fn acknowledge(&self, sequence: u64) -> Result<(), ApplicationError> {
        if self.0.authoritative_writer {
            self.0
                .storage
                .acknowledge_search_operations(sequence)
                .await?;
        }
        self.0.observed_sequence.store(sequence, Ordering::Release);
        self.0.committed_sequence.send_replace(sequence);
        Ok(())
    }

    /// Reclaim old generations after the last in-flight query releases its reader.
    fn clean_retired_generations(&self) -> Result<(), ApplicationError> {
        let mut retired = self.0.retired.lock().map_err(|_| poisoned_search())?;
        retired.retain(|(path, reader)| {
            if reader.upgrade().is_some() { return true; }
            match fs::remove_dir_all(path) {
                Ok(()) => false,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
                Err(error) => { tracing::debug!(%error, path = %path.display(), "search generation cleanup deferred"); true }
            }
        });
        Ok(())
    }

    /// Remove interrupted or retired index directories only while startup holds writer ownership.
    fn clean_startup_generations(&self) -> Result<(), ApplicationError> {
        let Some(directory) = &self.0.directory else {
            return Ok(());
        };
        let current = read_generation(directory)?;
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name == current {
                continue;
            }
            if let Some(identity) = name.strip_prefix("generation-")
                && Uuid::parse_str(identity).is_ok()
                && entry.file_type()?.is_dir()
                && let Err(error) = fs::remove_dir_all(entry.path())
            {
                tracing::warn!(%error, "could not reclaim obsolete search generation");
            }
        }
        Ok(())
    }

    /// Clone the current reader without holding the generation lock while executing queries.
    fn current(&self) -> Result<Arc<SearchIndex>, ApplicationError> {
        Ok(self
            .0
            .current
            .read()
            .map_err(|_| poisoned_search())?
            .clone())
    }

    /// Coalesce at most 1024 operations for 64 distinct messages before committing current state.
    /// Stop before an unprocessed identifier and acknowledge only the consumed prefix. Temporary
    /// readers check every page for durable pruning before queuing any index mutations.
    async fn apply_pending(
        &self,
        index: &SearchIndex,
        after: u64,
    ) -> Result<Option<(usize, u64)>, ApplicationError> {
        let mut watermark = after;
        let mut operation_count = 0;
        let mut identifiers = Vec::with_capacity(INDEX_BATCH_SIZE);
        let mut seen = HashSet::with_capacity(INDEX_BATCH_SIZE);
        'pages: while operation_count < MAXIMUM_COALESCED_OPERATION_COUNT {
            let (status, operations) = self
                .0
                .storage
                .search_batch(watermark, INDEX_BATCH_SIZE)
                .await?;
            if !self.0.authoritative_writer && status.indexed_sequence > after {
                return Ok(None);
            }
            let page_size = operations.len();
            for operation in operations {
                if !seen.contains(&operation.message_identifier) {
                    if identifiers.len() == INDEX_BATCH_SIZE {
                        break 'pages;
                    }
                    seen.insert(operation.message_identifier);
                    identifiers.push(operation.message_identifier);
                }
                watermark = operation.sequence;
                operation_count += 1;
            }
            if page_size < INDEX_BATCH_SIZE {
                break;
            }
        }
        if operation_count == 0 {
            return Ok(Some((0, after)));
        }
        let messages = self.0.storage.indexed_messages(&identifiers).await?;
        if !self.0.authoritative_writer
            && self.0.storage.search_status().await?.indexed_sequence > after
        {
            return Ok(None);
        }
        let documents: Vec<_> = messages.into_iter().map(indexed_document).collect();
        for document in &documents {
            seen.remove(&document.identifier);
        }
        let deleted: Vec<_> = identifiers
            .into_iter()
            .filter(|identifier| seen.contains(identifier))
            .collect();
        index.delete_batch(&deleted)?;
        index.upsert_batch(&documents)?;
        index.commit(watermark)?;
        Ok(Some((operation_count, watermark)))
    }
}

/// Convert the storage-owned index projection without exposing Tantivy outside its crate.
fn indexed_document(message: sandpost_storage::IndexedMessage) -> IndexedDocument {
    IndexedDocument {
        identifier: message.identifier,
        sequence: message.sequence,
        revision: message.revision,
        facts: message.facts,
    }
}

/// Reject a malformed CURRENT pointer rather than opening an arbitrary filesystem path.
fn read_generation(directory: &Path) -> Result<String, ApplicationError> {
    let generation = fs::read_to_string(directory.join("CURRENT"))?;
    let generation = generation.trim();
    let suffix = generation
        .strip_prefix("generation-")
        .ok_or_else(|| ApplicationError::Unavailable("invalid search generation pointer".into()))?;
    Uuid::parse_str(suffix)
        .map_err(|_| ApplicationError::Unavailable("invalid search generation identity".into()))?;
    Ok(generation.to_owned())
}

/// Durably publish a completed reader generation through a synced atomic pointer rename.
fn publish_generation(directory: &Path, generation: &str) -> Result<(), ApplicationError> {
    let temporary = directory.join(format!("CURRENT-{}", Uuid::new_v4()));
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    output.write_all(generation.as_bytes())?;
    output.sync_all()?;
    fs::rename(&temporary, directory.join("CURRENT"))?;
    #[cfg(unix)]
    File::open(directory)?.sync_all()?;
    Ok(())
}

/// Fail closed after poisoned search coordination rather than using partial state.
fn poisoned_search() -> ApplicationError {
    ApplicationError::Unavailable("search coordinator lock poisoned".into())
}

#[cfg(test)]
mod tests {
    //! Recovery, replay, and generation tests for the disposable search coordinator.
    use super::*;
    use sandpost_core::{Message, MessageFacts};
    use sandpost_query::Expression;
    use sandpost_storage_sqlite::SqliteStorage;

    /// Allocate a storage handle backed by an isolated in-memory database.
    async fn memory_storage() -> Arc<dyn Storage> {
        Arc::new(
            SqliteStorage::memory()
                .await
                .expect("in-memory storage should initialize"),
        )
    }

    /// Store one normalized fixture through the same outbox transaction as SMTP.
    async fn insert(storage: &Arc<dyn Storage>, subject: &str) -> MessageIdentifier {
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: subject.into(),
                ..MessageFacts::default()
            },
            raw_message: Vec::new(),
            attachments: Vec::new(),
        };
        storage.insert_message(&message).await.unwrap();
        message.identifier
    }

    /// Enqueue repeated replacements of one identifier to exercise outbox coalescing and replay.
    async fn insert_with_replayed_mutations(
        storage: &Arc<dyn Storage>,
        replacement_count: usize,
    ) -> Message {
        let message = Message {
            identifier: MessageIdentifier::new(),
            facts: MessageFacts {
                subject: "coalesced message".into(),
                headers: std::collections::BTreeMap::from([(
                    "x-burst".into(),
                    vec!["preserved header".into()],
                )]),
                ..MessageFacts::default()
            },
            raw_message: Vec::new(),
            attachments: Vec::new(),
        };
        storage.insert_message(&message).await.unwrap();
        for _ in 0..replacement_count {
            storage.delete_message(message.identifier).await.unwrap();
            storage.insert_message(&message).await.unwrap();
        }
        message
    }

    /// Read all fixture identifiers without involving a transport or database scan.
    fn identifiers(search: &SearchCoordinator) -> Vec<MessageIdentifier> {
        search
            .search(&MessageQuery {
                authorization: None,
                filter: Expression::True,
                message_identifier: None,
                before: None,
                limit: 100,
            })
            .unwrap()
    }

    /// Own a unique disposable data directory for durable index recovery tests.
    struct Directory(PathBuf);
    impl Drop for Directory {
        /// Reclaim only this test's randomly named temporary index data.
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// Allocate a collision-resistant test path without depending on repository data.
    fn directory() -> Directory {
        Directory(std::env::temp_dir().join(format!("sandpost-search-recovery-{}", Uuid::new_v4())))
    }

    /// Repeated replacements commit as two bounded message batches without early pruning.
    #[tokio::test]
    async fn repeated_message_operations_coalesce_without_acknowledging_the_next_message() {
        let directory = directory();
        let storage = memory_storage().await;
        let search = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let mut expected = Vec::new();
        for _ in 0..100 {
            expected.push(insert_with_replayed_mutations(&storage, 6).await.identifier);
        }
        assert_eq!(
            storage.search_status().await.unwrap().pending_operations,
            1300
        );
        assert_eq!(search.synchronize_batch().await.unwrap(), 64 * 13);
        let status = storage.search_status().await.unwrap();
        assert_eq!(status.indexed_sequence, 64 * 13);
        assert_eq!(status.pending_operations, 36 * 13);
        let mut first_batch = expected[..64].to_vec();
        first_batch.reverse();
        assert_eq!(identifiers(&search), first_batch);
        let (_, remaining) = storage
            .search_batch(status.indexed_sequence, 64)
            .await
            .unwrap();
        assert_eq!(remaining[0].message_identifier, expected[64]);
        assert_eq!(search.synchronize_batch().await.unwrap(), 36 * 13);
        expected.reverse();
        assert_eq!(identifiers(&search), expected);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 0);
    }

    /// Repeated mutations of one message remain bounded and replay preserves delete/reinsert.
    #[tokio::test]
    async fn operation_limit_preserves_unacknowledged_replay_and_newer_reinserted_revision() {
        let directory = directory();
        let storage = memory_storage().await;
        let search = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let mut message =
            insert_with_replayed_mutations(&storage, MAXIMUM_COALESCED_OPERATION_COUNT / 2).await;
        let index = search.current().unwrap();
        let (count, watermark) = search.apply_pending(&index, 0).await.unwrap().unwrap();
        assert_eq!(count, MAXIMUM_COALESCED_OPERATION_COUNT);
        assert_eq!(watermark, MAXIMUM_COALESCED_OPERATION_COUNT as u64);
        assert_eq!(storage.search_status().await.unwrap().indexed_sequence, 0);
        assert_eq!(identifiers(&search), [message.identifier]);
        assert_eq!(search.synchronize_batch().await.unwrap(), count);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 1);

        let previous_revision = storage
            .indexed_messages(&[message.identifier])
            .await
            .unwrap()[0]
            .revision;
        storage.delete_message(message.identifier).await.unwrap();
        message.facts.subject = "reinserted current message".into();
        message.facts.headers.clear();
        storage.insert_message(&message).await.unwrap();
        assert_eq!(search.synchronize_batch().await.unwrap(), 3);
        let current = storage
            .indexed_messages(&[message.identifier])
            .await
            .unwrap();
        assert!(current[0].revision > previous_revision);
        let hits = search
            .search_hits(&MessageQuery {
                authorization: None,
                filter: Expression::True,
                message_identifier: Some(message.identifier),
                before: None,
                limit: 1,
            })
            .unwrap();
        assert_eq!(hits, [(message.identifier, current[0].revision)]);
        let summaries = storage.hydrate_current_messages(&hits).await.unwrap();
        assert_eq!(summaries[0].subject, message.facts.subject);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 0);
        storage.delete_message(message.identifier).await.unwrap();
        search.catch_up().await.unwrap();
        assert!(identifiers(&search).is_empty());
    }

    /// A Tantivy commit followed by a lost storage acknowledgement can replay without duplication.
    #[tokio::test]
    async fn commit_before_acknowledgement_replays_idempotently_and_deletions_converge() {
        let storage = memory_storage().await;
        let search = SearchCoordinator::memory(storage.clone()).await.unwrap();
        let identifier = insert(&storage, "replay").await;
        let (_, committed) = search
            .apply_pending(&search.current().unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(storage.search_status().await.unwrap().indexed_sequence, 0);
        assert_eq!(identifiers(&search), [identifier]);
        assert_eq!(
            search.current().unwrap().committed_sequence().unwrap(),
            committed
        );
        search.synchronize_batch().await.unwrap();
        assert_eq!(identifiers(&search), [identifier]);
        assert_eq!(search.status().await.unwrap().pending_operations, 0);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 1);
        storage.delete_message(identifier).await.unwrap();
        assert_eq!(identifiers(&search), [identifier]);
        search.catch_up().await.unwrap();
        assert!(identifiers(&search).is_empty());
        assert!(storage.get_message(identifier).await.unwrap().is_none());
    }

    /// Missing or corrupt generations rebuild from storage even after old operations were pruned.
    #[tokio::test]
    async fn deleted_and_corrupt_tantivy_state_is_disposable_and_storage_remains_authoritative() {
        let directory = directory();
        let storage = memory_storage().await;
        let identifier = insert(&storage, "authoritative").await;
        let search = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        assert_eq!(identifiers(&search), [identifier]);
        assert!(
            SearchCoordinator::open(storage.clone(), directory.0.clone())
                .await
                .is_err()
        );
        let generation = read_generation(&directory.0).unwrap();
        drop(search);
        fs::remove_dir_all(directory.0.join(generation)).unwrap();
        let rebuilt = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        assert_eq!(identifiers(&rebuilt), [identifier]);
        let generation = read_generation(&directory.0).unwrap();
        drop(rebuilt);
        fs::write(
            directory.0.join(generation).join("meta.json"),
            "broken index metadata",
        )
        .unwrap();
        let recovered = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        assert_eq!(identifiers(&recovered), [identifier]);
        assert_eq!(
            storage
                .get_message_metadata(identifier)
                .await
                .unwrap()
                .unwrap()
                .0
                .subject,
            "authoritative"
        );
    }

    /// Restart after an index commit and before storage acknowledgement using durable database state.
    #[tokio::test]
    async fn restart_replays_a_committed_unacknowledged_batch_against_persistent_storage() {
        let directory = directory();
        fs::create_dir_all(&directory.0).unwrap();
        let database = directory.0.join("mail.sqlite3");
        let index_directory = directory.0.join("search");
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&database).await.unwrap());
        let search = SearchCoordinator::open(storage.clone(), index_directory.clone())
            .await
            .unwrap();
        let identifier = insert_with_replayed_mutations(&storage, 50)
            .await
            .identifier;
        search
            .apply_pending(&search.current().unwrap(), 0)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(search.status().await.unwrap().pending_operations, 101);
        drop(search);
        drop(storage);
        let reopened: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&database).await.unwrap());
        let recovered = SearchCoordinator::open(reopened.clone(), index_directory)
            .await
            .unwrap();
        assert_eq!(identifiers(&recovered), [identifier]);
        assert_eq!(
            reopened.search_status().await.unwrap().pending_operations,
            0
        );
    }

    /// A temporary index cannot consume operations required by the durable installation index.
    #[tokio::test]
    async fn independent_temporary_index_does_not_prune_the_durable_outbox() {
        let directory = directory();
        let storage = memory_storage().await;
        let durable = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let identifier = insert(&storage, "not acknowledged by temporary index").await;
        let temporary = SearchCoordinator::memory(storage.clone()).await.unwrap();
        assert_eq!(identifiers(&temporary), [identifier]);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 1);
        durable.catch_up().await.unwrap();
        assert_eq!(identifiers(&durable), [identifier]);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 0);
        let later = insert(&storage, "second operation").await;
        durable.catch_up().await.unwrap();
        temporary.catch_up().await.unwrap();
        assert_eq!(identifiers(&temporary), [later, identifier]);
    }

    /// Recover pruned operations before processing a retained delete without consuming storage work.
    #[tokio::test]
    async fn temporary_synchronization_recovers_pruned_gap_and_pending_deletion() {
        let directory = directory();
        let storage = memory_storage().await;
        let removed = insert(&storage, "removed").await;
        let durable = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let temporary = SearchCoordinator::memory(storage.clone()).await.unwrap();
        let observed = temporary.status().await.unwrap().indexed_sequence;
        let added = insert(&storage, "added and acknowledged").await;
        durable.catch_up().await.unwrap();
        storage.delete_message(removed).await.unwrap();
        assert!(
            temporary
                .apply_pending(&temporary.current().unwrap(), observed)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(temporary.status().await.unwrap().indexed_sequence, observed);
        temporary.catch_up().await.unwrap();
        assert_eq!(identifiers(&temporary), [added]);
        assert_eq!(temporary.status().await.unwrap().pending_operations, 0);
        assert_eq!(storage.search_status().await.unwrap().pending_operations, 1);
        durable.catch_up().await.unwrap();
        assert_eq!(identifiers(&durable), [added]);
    }

    /// Explicit closure releases both writer locks despite retained coordinator and reader handles.
    #[tokio::test]
    async fn closed_writer_allows_durable_restart_with_retained_readers() {
        let directory = directory();
        let storage = memory_storage().await;
        let original = insert(&storage, "before shutdown").await;
        let search = SearchCoordinator::open(storage.clone(), directory.0.clone())
            .await
            .unwrap();
        let retained = search.clone();
        let reader = search.current().unwrap();
        assert!(
            SearchCoordinator::open(storage.clone(), directory.0.clone())
                .await
                .is_err()
        );
        search.close_writer().await.unwrap();
        search.close_writer().await.unwrap();
        assert!(retained.synchronize_batch().await.is_err());
        assert!(retained.rebuild().await.is_err());
        assert!(reader.delete(original).is_err());
        assert_eq!(identifiers(&retained), [original]);
        let later = insert(&storage, "after shutdown").await;
        let restarted = SearchCoordinator::open(storage, directory.0.clone())
            .await
            .unwrap();
        assert_eq!(identifiers(&restarted), [later, original]);
        assert_eq!(identifiers(&retained), [original]);
    }

    /// Commit and publication wake progress waiters; closing wakes them with an error.
    #[tokio::test]
    async fn search_progress_waits_for_reader_commit_and_wakes_on_closure() {
        use std::time::Duration;
        use tokio::time::timeout;

        let storage = memory_storage().await;
        let search = SearchCoordinator::memory(storage.clone()).await.unwrap();
        let first = insert(&storage, "batch progress").await;
        let sequence = storage.search_status().await.unwrap().latest_sequence;
        let wait = search.wait_for_sequence(sequence);
        tokio::pin!(wait);
        assert!(timeout(Duration::from_millis(10), &mut wait).await.is_err());
        search.synchronize_batch().await.unwrap();
        timeout(Duration::from_secs(1), &mut wait)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(identifiers(&search), [first]);

        let second = insert(&storage, "rebuild progress").await;
        let sequence = storage.search_status().await.unwrap().latest_sequence;
        let wait = search.wait_for_sequence(sequence);
        tokio::pin!(wait);
        assert!(timeout(Duration::from_millis(10), &mut wait).await.is_err());
        search.rebuild().await.unwrap();
        timeout(Duration::from_secs(1), &mut wait)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(identifiers(&search), [second, first]);

        let wait = search.wait_for_sequence(sequence + 1);
        tokio::pin!(wait);
        assert!(timeout(Duration::from_millis(10), &mut wait).await.is_err());
        search.close_writer().await.unwrap();
        assert!(
            timeout(Duration::from_secs(1), &mut wait)
                .await
                .unwrap()
                .is_err()
        );
        assert!(search.wait_for_sequence(0).await.is_err());
    }

    /// Rebuild streams multiple bounded pages and reproduces newest-first search keysets.
    #[tokio::test]
    async fn rebuild_spans_batch_boundaries_and_keeps_sequence_pagination_stable() {
        let storage = memory_storage().await;
        let mut expected = Vec::new();
        for ordinal in 0..145 {
            expected.push(insert(&storage, &format!("batch message {ordinal}")).await);
        }
        expected.reverse();
        let search = SearchCoordinator::memory(storage.clone()).await.unwrap();
        search.rebuild().await.unwrap();
        let mut found = Vec::new();
        let mut before = None;
        loop {
            let page = search
                .search(&MessageQuery {
                    authorization: None,
                    filter: Expression::True,
                    message_identifier: None,
                    before,
                    limit: 23,
                })
                .unwrap();
            if page.is_empty() {
                break;
            }
            let summaries = storage.hydrate_messages(&page).await.unwrap();
            before = summaries.last().map(|summary| summary.sequence);
            found.extend(page);
        }
        assert_eq!(found, expected);
        assert_eq!(search.status().await.unwrap().pending_operations, 0);
    }

    /// A retired generation remains available until its last in-flight reader finishes.
    #[tokio::test]
    async fn publication_reclaims_old_generations_after_readers_release_them() {
        let directory = directory();
        let storage = memory_storage().await;
        insert(&storage, "reader").await;
        let search = SearchCoordinator::open(storage, directory.0.clone())
            .await
            .unwrap();
        let original_path = directory.0.join(read_generation(&directory.0).unwrap());
        let in_flight_reader = search.current().unwrap();
        search.rebuild().await.unwrap();
        assert!(original_path.exists());
        assert_eq!(identifiers(&search).len(), 1);
        drop(in_flight_reader);
        search.synchronize_batch().await.unwrap();
        assert!(!original_path.exists());
    }
}
