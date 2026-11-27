//! A Mono's transcript renders each decision as its own input entity.
use super::*;
use crate::{
    monos::cards::CardAction,
    questions::{Answered, QuestionCard},
};
use gpui::AnyElement;
use monocode_core::{agent::Question, mono::cards::DecisionRecord};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
pub(super) struct Decisions {
    records: BTreeMap<String, DecisionRecord>,
    restored: TranscriptDecisions,
    release_at: Option<u64>,
    cards: HashMap<String, (Entity<QuestionCard>, gpui::Subscription)>,
}

#[derive(Default)]
struct TranscriptDecisions {
    records: BTreeMap<String, DecisionRecord>,
    legacy: HashSet<String>,
    replied: HashSet<String>,
}

// Before answers carried card ids, clicking a choice queued its exact label.
// Attribute at most one reply to the most recent matching card in that turn.
fn legacy_records(
    rows: &[Row],
    existing: &BTreeMap<String, DecisionRecord>,
    queued_reply: Option<&str>,
    release_at: Option<u64>,
) -> TranscriptDecisions {
    use monocode_core::mono::cards::question;

    fn record_reply(
        offers: &mut Vec<Question>,
        records: &mut BTreeMap<String, DecisionRecord>,
        reply: &str,
    ) {
        let reply = reply.trim();
        if let Some(card) = offers.iter().rev().find(|card| {
            card.items.iter().any(|item| {
                item.options
                    .iter()
                    .any(|option| option.label.trim() == reply)
            })
        }) && let Some(record) = records.get_mut(&card.id)
        {
            record.answers = vec![reply.to_owned()];
        }
        offers.clear();
    }

    let mut out = TranscriptDecisions::default();
    let mut offers = Vec::new();
    let mut legacy_pending = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        if row.role == "mono-card"
            && let Ok(value) = serde_json::from_str::<Value>(&row.text)
            && let Some(card) = question(&value)
        {
            let modern = value["decisionVersion"] == 1
                || row
                    .sent_at
                    .zip(release_at)
                    .is_some_and(|(sent, release)| sent >= release);
            if !modern {
                out.legacy.insert(card.id.clone());
                legacy_pending.push(card.id.clone());
            }
            if existing
                .get(&card.id)
                .is_some_and(|record| !record.answers.is_empty())
            {
                continue;
            }
            if let Some(answers) = value["decisionAnswers"].as_array()
                && let Ok(answers) = monocode_core::mono::cards::decision_answers(answers)
            {
                out.records.insert(
                    card.id.clone(),
                    DecisionRecord {
                        question: card.title,
                        answers,
                        updated_at: row.sent_at,
                    },
                );
            } else if !existing.contains_key(&card.id) {
                out.records
                    .entry(card.id.clone())
                    .or_insert_with(|| DecisionRecord {
                        question: card.title.clone(),
                        updated_at: modern.then_some(row.sent_at.unwrap_or_default()),
                        ..Default::default()
                    });
                if !modern {
                    offers.push(card);
                }
            }
        } else if row.role == "user" && row.first && !legacy_pending.is_empty() {
            out.replied.extend(legacy_pending.drain(..));
            if !offers.is_empty() {
                let reply = rows[ix + 1..].iter().take_while(|row| !row.first).fold(
                    row.text.clone(),
                    |mut text, row| {
                        text.push_str(&row.text);
                        text
                    },
                );
                record_reply(&mut offers, &mut out.records, &reply);
            }
        }
    }
    if let Some(reply) = queued_reply {
        record_reply(&mut offers, &mut out.records, reply);
    }
    out
}

impl TranscriptView {
    /// Roster restoration includes decisions that precede the loaded transcript tail.
    pub(crate) fn set_mono_decisions(
        &mut self,
        records: BTreeMap<String, DecisionRecord>,
        release_at: Option<u64>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.mono_decisions.records == records && self.mono_decisions.release_at == release_at {
            return false;
        }
        self.mono_decisions.cards.retain(|id, _| {
            records
                .get(id)
                .is_none_or(|record| record.answers.is_empty())
        });
        self.mono_decisions.records = records;
        self.mono_decisions.release_at = release_at;
        self.refresh_legacy_mono_decisions();
        cx.notify();
        true
    }

    pub(super) fn refresh_legacy_mono_decisions(&mut self) {
        if self.mono {
            self.mono_decisions.restored = legacy_records(
                &self.source,
                &self.mono_decisions.records,
                self.pending
                    .first()
                    .filter(|reply| reply.error.is_none())
                    .map(|reply| reply.text.as_ref()),
                self.mono_decisions.release_at,
            );
        }
    }

    pub(crate) fn pending_mono_decisions(&self) -> BTreeMap<String, String> {
        self.pending_decisions()
            .map(|(id, record)| (id.clone(), record.question.clone()))
            .collect()
    }

    pub(crate) fn mono_decisions_need_attention(&self) -> bool {
        self.mono_attention_decisions().next().is_some()
    }

    pub(crate) fn mono_attention_decisions(
        &self,
    ) -> impl Iterator<Item = (&String, &DecisionRecord)> {
        self.pending_decisions().filter(|(id, record)| {
            record.updated_at.is_some() && !self.mono_decisions.restored.legacy.contains(*id)
        })
    }

    fn pending_decisions(&self) -> impl Iterator<Item = (&String, &DecisionRecord)> {
        let restored = &self.mono_decisions.restored;
        self.mono_decisions
            .records
            .iter()
            .filter(|(id, _)| !restored.records.contains_key(*id))
            .chain(restored.records.iter())
            .filter(|(id, record)| {
                self.mono && record.answers.is_empty() && !restored.replied.contains(*id)
            })
    }

    pub(super) fn mono_decision(&mut self, value: &Value, cx: &mut Context<Self>) -> AnyElement {
        let Some(question) = monocode_core::mono::cards::question(value) else {
            return div().into_any_element();
        };
        let id = question.id.clone();
        let selector = format!("mono-decision-{id}");
        let mut row = div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .w_full()
            .max_w(px(480.));
        let answer = self
            .mono_decisions
            .records
            .get(&id)
            .filter(|r| !r.answers.is_empty())
            .or_else(|| {
                self.mono_decisions
                    .restored
                    .records
                    .get(&id)
                    .filter(|r| !r.answers.is_empty())
            })
            .map(|record| format!("You chose: {}", record.answers.join(" · ")))
            .or_else(|| {
                self.mono_decisions
                    .restored
                    .replied
                    .contains(&id)
                    .then(|| "Answered in chat".into())
            });
        if let Some(answer) = answer {
            row = row.child(
                div()
                    .p_3()
                    .rounded_lg()
                    .border_1()
                    .border_color(content(0.12))
                    .bg(content(0.035))
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(div().text_size(px(13.)).child(question.title))
                    .child(
                        div()
                            .flex()
                            .items_start()
                            .gap_1p5()
                            .text_size(px(12.))
                            .text_color(content(0.65))
                            .child(icon("check", 14.).text_color(success_color()))
                            .child(answer),
                    ),
            );
        } else {
            row = row.child(self.decision_control(id, question, cx));
        }
        row.into_any_element()
    }

    fn decision_control(
        &mut self,
        id: String,
        question: Question,
        cx: &mut Context<Self>,
    ) -> Entity<QuestionCard> {
        let (control, _) = self.mono_decisions.cards.entry(id).or_insert_with(|| {
            let control = cx.new(|cx| QuestionCard::inline(question, cx));
            let answer = cx.subscribe(&control, |_, _, event: &Answered, cx| {
                if let Some(answers) = event.answers.as_ref().and_then(|groups| groups.first()) {
                    cx.emit(CardAction(json!({
                        "type": "decision-answer",
                        "id": event.id,
                        "answers": answers
                    })));
                }
            });
            (control, answer)
        });
        control.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_choice_replies_resolve_only_one_matching_card() {
        let row = |role: &str, text: String| Row {
            role: role.into(),
            text,
            first: true,
            sent_at: None,
        };
        let offer = |id: &str| {
            row(
                "mono-card",
                json!({"type":"choices","id":id,"options":["Run tests", "Skip"]}).to_string(),
            )
        };
        let rows = [
            offer("first"),
            offer("second"),
            row("user", "Run tests".into()),
            offer("third"),
            row("user", "Unrelated message".into()),
            offer("fourth"),
        ];
        let resolved = legacy_records(&rows, &BTreeMap::new(), Some("Skip"), None);
        let records = resolved.records;
        assert!(records["first"].answers.is_empty());
        assert_eq!(records["second"].answers, ["Run tests"]);
        assert!(records["third"].answers.is_empty());
        assert_eq!(records["fourth"].answers, ["Skip"]);
        assert!(resolved.replied.contains("first"));
        assert!(resolved.replied.contains("second"));
        assert!(resolved.replied.contains("third"));
        assert!(!resolved.replied.contains("fourth"));
    }

    #[test]
    fn a_typed_reply_resolves_old_cards_without_clearing_new_attention() {
        let offer = |id: &str, at| Row {
            role: "mono-card".into(),
            text: json!({"type":"choices","id":id,"options":["Run", "Skip"]}).to_string(),
            first: true,
            sent_at: Some(at),
        };
        let rows = [
            offer("legacy", 10),
            offer("modern", 25),
            Row {
                role: "user".into(),
                text: "Use the safe defaults".into(),
                first: true,
                sent_at: Some(30),
            },
        ];
        let mut view = TranscriptView::new();
        view.mono = true;
        view.mono_decisions.restored = legacy_records(&rows, &BTreeMap::new(), None, Some(20));
        assert!(!view.pending_mono_decisions().contains_key("legacy"));
        assert_eq!(
            view.mono_attention_decisions()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            ["modern"]
        );
    }

    #[test]
    fn a_pruned_modern_card_keeps_its_attached_answer() {
        let rows = [Row {
            role: "mono-card".into(),
            text: json!({"type":"choices","id":"resolved","options":["Run"],"decisionVersion":1,"decisionAnswers":["Run"]}).to_string(),
            first: true,
            sent_at: Some(25),
        }];
        let restored = legacy_records(&rows, &BTreeMap::new(), None, Some(20));
        assert_eq!(restored.records["resolved"].answers, ["Run"]);
    }
}
