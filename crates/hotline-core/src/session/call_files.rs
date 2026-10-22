//! Call work delivers its recorded mutations without asking the model to send them.
use super::{Post, Room, files, new_id, now_ms};
use crate::contract::{Attachment, ChangedFile, NoticeLevel, ThreadId, TranscriptEvent};
use crate::driver::CapabilityLease;
use crate::sent;

pub(super) struct CallFile {
    pub file: sent::Prepared,
    pub origin: String,
    pub created: Option<bool>,
    pub caption: String,
}

impl Room {
    pub(super) async fn post_call_files(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        changes: &[ChangedFile],
        capability: &CapabilityLease,
    ) {
        if capability.check().is_err()
            || changes.is_empty()
                && super::lock(&self.posting)
                    .get(thread)
                    .is_none_or(|turn| turn.files.is_empty())
        {
            return;
        }
        let result = self
            .prepare_call_files(persona_id, thread, changes, capability)
            .await;
        if let Err(error) = result {
            self.threads().write(
                thread,
                persona_id,
                &TranscriptEvent::Notice {
                    id: new_id(),
                    ts: now_ms(),
                    level: NoticeLevel::Warn,
                    text: format!("The call's files could not all be posted: {error}"),
                },
            );
        }
    }

    async fn prepare_call_files(
        &self,
        persona_id: &str,
        thread: &ThreadId,
        changes: &[ChangedFile],
        capability: &CapabilityLease,
    ) -> Result<(), String> {
        let persona = self.persona(persona_id)?;
        // The batch belongs to one work turn. Later corrections to the same
        // path are new results; an earlier attachment must not hide them.
        let mut sent_origins = Vec::new();
        let pending = super::lock(&self.posting)
            .get_mut(thread)
            .map(|turn| std::mem::take(&mut turn.files))
            .unwrap_or_default();
        let mut ready: Vec<CallFile> = Vec::new();
        for file in pending {
            if let Some(previous) = ready
                .iter_mut()
                .find(|previous| previous.origin == file.origin)
            {
                *previous = file;
            } else {
                sent_origins.push(file.origin.clone());
                ready.push(file);
            }
        }
        let mut errors = Vec::new();
        for change in changes {
            capability.check()?;
            match files::from_workspace(
                &persona,
                self.log.root(),
                &change.path,
                Some(capability.clone()),
                true,
            )
            .await
            {
                Ok(taken) => {
                    if let Some(file) = ready.iter_mut().find(|file| file.origin == taken.origin) {
                        file.created = Some(file.created.unwrap_or(false) || change.created);
                        // A send before the last edit joins the final result,
                        // rather than freezing an intermediate draft.
                        match files::prepare(taken.name, taken.bytes, false).await {
                            Ok(latest) => file.file = latest,
                            Err(error) => errors.push(error),
                        }
                        continue;
                    }
                    if sent_origins.contains(&taken.origin) {
                        continue;
                    }
                    sent_origins.push(taken.origin.clone());
                    match files::prepare(taken.name, taken.bytes, false).await {
                        Ok(file) => ready.push(CallFile {
                            file,
                            origin: taken.origin,
                            created: Some(change.created),
                            caption: String::new(),
                        }),
                        Err(error) => errors.push(error),
                    }
                }
                Err(error) => errors.push(error),
            }
        }
        let grouped = ready.len() > 3;
        let batch_size = if grouped { ready.len() } else { 1 };
        for batch in ready.chunks(batch_size) {
            let id = new_id();
            let mut attachments = Vec::new();
            let written = (|| {
                for (index, item) in batch.iter().enumerate() {
                    let file = &item.file;
                    capability.check()?;
                    let path = sent::store_indexed(self.log.root(), persona_id, &id, index, file)
                        .map_err(|error| error.to_string())?;
                    attachments.push(Attachment {
                        created: item.created,
                        kind: file.kind,
                        name: file.name.clone(),
                        path: path.display().to_string(),
                        mime_type: Some(file.mime_type.clone()),
                        size: Some(file.bytes.len() as i64),
                        width: file.dimensions.map(|(w, _)| w),
                        height: file.dimensions.map(|(_, h)| h),
                        origin: Some(item.origin.clone()),
                    });
                }
                capability.check()?;
                self.post(
                    persona_id,
                    thread,
                    Post {
                        id: id.clone(),
                        text: if grouped {
                            format!("{} files from the call", batch.len())
                        } else {
                            batch[0].caption.clone()
                        },
                        attachments: Some(attachments),
                        scheduled: None,
                        sources: Vec::new(),
                    },
                )
            })();
            if let Err(error) = written {
                sent::discard(self.log.root(), persona_id, &id);
                return Err(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }
}
