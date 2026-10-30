use std::sync::{Mutex, OnceLock};

use pi_durable::cancel::CancelToken;
use pi_durable::entries::{assistant_entry, user_entry};
use pi_durable::errors::{Error, Result};
use pi_durable::ids::{ConversationId, DocumentId, EntryId, Seq, SubmissionId, TaskId};
use pi_durable::storage::memory::MemoryStorage;
use pi_durable::storage::sqlite::SqliteStorage;
use pi_durable::types::{
    AnyTaskRecord, CommittedEntry, ConversationQuery, ConversationRecord, Cursor, DocumentAddress,
    DocumentPoint, DocumentQuery, DocumentRecord, EntryQuery, EntryRecord, HeadMarker, Page,
    Storage, StorageWrite, StoredDocument, SubmissionQuery, SubmissionRecord, TableChange,
    TaskQuery,
};

pub struct Deferred {
    path: String,
    memory: MemoryStorage,
    log: Mutex<Vec<(Seq, Vec<StorageWrite>)>>,
    file: OnceLock<SqliteStorage>,
}

fn rejected(message: String) -> Error {
    Error::StorageRejected {
        message,
        cause: None,
    }
}

fn starts_conversation(write: &StorageWrite) -> bool {
    let StorageWrite::Table(change) = write else {
        return false;
    };
    let TableChange::Entry(entry) = change.as_ref() else {
        return false;
    };
    [user_entry(), assistant_entry()]
        .iter()
        .any(|kind| kind.kind() == entry.kind)
}

fn remove(path: &str) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(format!("{path}{suffix}"));
    }
}

impl Deferred {
    pub fn new(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            memory: MemoryStorage::new(),
            log: Mutex::new(Vec::new()),
            file: OnceLock::new(),
        }
    }

    fn current(&self) -> &dyn Storage {
        match self.file.get() {
            Some(file) => file,
            None => &self.memory,
        }
    }

    fn create(
        &self,
        log: &[(Seq, Vec<StorageWrite>)],
        writes: &[StorageWrite],
        cancel: &CancelToken,
    ) -> Result<(SqliteStorage, Seq)> {
        let file = SqliteStorage::open(&self.path, &Default::default())?;
        for (seq, batch) in log {
            let replayed = file.commit(batch, cancel)?;
            if replayed != *seq {
                return Err(rejected(format!(
                    "{} replayed commit {} as {}",
                    self.path, seq.0, replayed.0
                )));
            }
        }
        let seq = file.commit(writes, cancel)?;
        Ok((file, seq))
    }
}

impl Storage for Deferred {
    fn commit(&self, writes: &[StorageWrite], cancel: &CancelToken) -> Result<Seq> {
        let mut log = self
            .log
            .lock()
            .map_err(|_| rejected(format!("{} is poisoned", self.path)))?;
        if let Some(file) = self.file.get() {
            return file.commit(writes, cancel);
        }
        if !writes.iter().any(starts_conversation) {
            let seq = self.memory.commit(writes, cancel)?;
            log.push((seq, writes.to_vec()));
            return Ok(seq);
        }
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(|e| rejected(format!("{} could not be created: {e}", self.path)))?;
        match self.create(&log, writes, cancel) {
            Ok((file, seq)) => {
                log.clear();
                let _ = self.file.set(file);
                Ok(seq)
            }
            Err(error) => {
                remove(&self.path);
                Err(error)
            }
        }
    }

    fn mint_id(&self) -> Result<u64> {
        self.memory.mint_id()
    }

    fn conversation(
        &self,
        id: ConversationId,
        cancel: &CancelToken,
    ) -> Result<Option<ConversationRecord>> {
        self.current().conversation(id, cancel)
    }

    fn scan_conversations(
        &self,
        query: &ConversationQuery,
        limit: usize,
        cursor: Option<&Cursor>,
        cancel: &CancelToken,
    ) -> Result<Page<ConversationRecord>> {
        self.current()
            .scan_conversations(query, limit, cursor, cancel)
    }

    fn entry(&self, id: EntryId, cancel: &CancelToken) -> Result<Option<CommittedEntry>> {
        self.current().entry(id, cancel)
    }

    fn visible_entry(
        &self,
        conversation_id: ConversationId,
        id: EntryId,
        cancel: &CancelToken,
    ) -> Result<Option<CommittedEntry>> {
        self.current().visible_entry(conversation_id, id, cancel)
    }

    fn find_latest_head_marker(
        &self,
        conversation_id: ConversationId,
        at_or_before: Option<EntryId>,
        cancel: &CancelToken,
    ) -> Result<Option<HeadMarker>> {
        self.current()
            .find_latest_head_marker(conversation_id, at_or_before, cancel)
    }

    fn scan_entries(
        &self,
        query: &EntryQuery,
        limit: usize,
        cursor: Option<&Cursor>,
        cancel: &CancelToken,
    ) -> Result<Page<EntryRecord>> {
        self.current().scan_entries(query, limit, cursor, cancel)
    }

    fn task(&self, id: TaskId, cancel: &CancelToken) -> Result<Option<AnyTaskRecord>> {
        self.current().task(id, cancel)
    }

    fn scan_tasks(
        &self,
        query: &TaskQuery,
        limit: usize,
        cursor: Option<&Cursor>,
        cancel: &CancelToken,
    ) -> Result<Page<AnyTaskRecord>> {
        self.current().scan_tasks(query, limit, cursor, cancel)
    }

    fn submission(
        &self,
        id: SubmissionId,
        cancel: &CancelToken,
    ) -> Result<Option<SubmissionRecord>> {
        self.current().submission(id, cancel)
    }

    fn scan_submissions(
        &self,
        query: &SubmissionQuery,
        limit: usize,
        cursor: Option<&Cursor>,
        cancel: &CancelToken,
    ) -> Result<Page<SubmissionRecord>> {
        self.current()
            .scan_submissions(query, limit, cursor, cancel)
    }

    fn submission_by_request(
        &self,
        conversation_id: ConversationId,
        request_id: &str,
        cancel: &CancelToken,
    ) -> Result<Option<SubmissionRecord>> {
        self.current()
            .submission_by_request(conversation_id, request_id, cancel)
    }

    fn find_document(
        &self,
        address: &DocumentAddress,
        at: DocumentPoint,
        cancel: &CancelToken,
    ) -> Result<Option<DocumentRecord>> {
        self.current().find_document(address, at, cancel)
    }

    fn document(
        &self,
        id: DocumentId,
        at: DocumentPoint,
        cancel: &CancelToken,
    ) -> Result<Option<StoredDocument>> {
        self.current().document(id, at, cancel)
    }

    fn scan_documents(
        &self,
        query: &DocumentQuery,
        limit: usize,
        cursor: Option<&Cursor>,
        cancel: &CancelToken,
    ) -> Result<Page<DocumentRecord>> {
        self.current().scan_documents(query, limit, cursor, cancel)
    }

    fn close(&self, cancel: &CancelToken) -> Result<()> {
        let closed = self.file.get().map(|file| file.close(cancel));
        self.memory.close(cancel)?;
        closed.unwrap_or(Ok(()))
    }
}
