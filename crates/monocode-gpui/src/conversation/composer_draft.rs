//! The text and paths left in a local composer, independent of its follow-ups.
use super::*;
use crate::ui_store_saves;
use monocode_core::store::ComposerDraft;

pub(crate) struct Changed(pub(crate) bool);
impl EventEmitter<Changed> for Conversation {}

#[derive(Default)]
pub(super) struct State {
    last: Option<ComposerDraft>,
    edited: bool,
}

fn local_session(summary: &Session) -> bool {
    summary.id > 0 && !summary.cwd.starts_with("remote://")
}

pub(super) fn restore(summary: &Session, store: &Database, cx: &mut Context<Conversation>) {
    if !local_session(summary) {
        return;
    }
    let id = summary.id;
    let latest = ui_store_saves::composer_latest(store, id, cx);
    let store = store.clone();
    let load = cx.background_executor().spawn(async move {
        let value = match latest {
            Some(value) => Some(value),
            None => locked(&store)?.read_composer_draft(id)?,
        };
        let Some(value) = value else {
            return Ok::<_, String>(None);
        };
        let draft: Option<ComposerDraft> =
            serde_json::from_str(&value).map_err(|error| error.to_string())?;
        Ok(draft.map(|draft| {
            let attachments: Vec<_> = draft
                .attachments
                .iter()
                .filter(|path| path.exists())
                .cloned()
                .collect();
            (draft, attachments)
        }))
    });
    cx.spawn(async move |this, cx| match load.await {
        Ok(Some((draft, attachments))) => {
            let _ = this.update(cx, |this, cx| {
                if this.summary.id != id
                    || this.is_remote()
                    || this.composer_draft.edited
                    || this.editing_last_turn.pending()
                    || !this.input.read(cx).content.is_empty()
                    || !this.attachments.is_empty()
                {
                    return;
                }
                this.attachments = attachments;
                this.input
                    .update(cx, |input, cx| input.set_text(draft.text.clone(), cx));
                this.composer_draft.last = Some(draft);
                this.save_composer_draft(cx);
                cx.emit(Changed(this.has_composer_draft()));
                cx.notify();
            });
        }
        Ok(None) => {}
        Err(error) => log::warn!("Could not restore a composer draft: {error}"),
    })
    .detach();
}

impl Conversation {
    pub(crate) fn has_composer_draft(&self) -> bool {
        self.composer_draft.last.is_some()
    }

    pub(super) fn save_composer_draft(&mut self, cx: &mut Context<Self>) {
        if !local_session(&self.summary) || self.editing_last_turn.pending() {
            return;
        }
        let input = self.input.read(cx);
        if input.is_composing() {
            return;
        }
        let text = input.content.as_ref();
        let empty = text.trim().is_empty() && self.attachments.is_empty();
        if (empty && self.composer_draft.last.is_none())
            || self.composer_draft.last.as_ref().is_some_and(|last| {
                !empty && last.text == text && last.attachments == self.attachments
            })
        {
            return;
        }
        let draft = (!empty).then(|| ComposerDraft {
            text: text.into(),
            attachments: self.attachments.clone(),
        });
        let saved = serde_json::to_string(&draft)
            .map_err(|error| error.to_string())
            .and_then(|value| {
                ui_store_saves::composer_saver(&self.store, self.summary.id, cx).save_latest(value)
            });
        match saved {
            Ok(ticket) => {
                let changed = draft.is_some() != self.has_composer_draft();
                self.composer_draft.last = draft;
                self.composer_draft.edited = true;
                if changed {
                    cx.emit(Changed(self.has_composer_draft()));
                }
                cx.background_executor()
                    .spawn(async move {
                        if let Ok(Err(error)) = ticket.result.recv().await {
                            log::warn!("Could not save a composer draft: {error}");
                        }
                    })
                    .detach();
            }
            Err(error) => log::warn!("Could not queue a composer draft: {error}"),
        }
    }
}
