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
    legacy: BTreeMap<String, DecisionRecord>,
    cards: HashMap<String, (Entity<QuestionCard>, gpui::Subscription)>,
}

// Before answers carried card ids, clicking a choice queued its exact label.
// Attribute at most one reply to the most recent matching card in that turn.
fn legacy_records(
    rows: &[Row],
    existing: &BTreeMap<String, DecisionRecord>,
    queued_reply: Option<&str>,
) -> BTreeMap<String, DecisionRecord> {
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

    let mut records = BTreeMap::new();
    let mut offers = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        if row.role == "mono-card"
            && let Ok(card) = serde_json::from_str::<Value>(&row.text)
            && let Some(card) = question(&card)
            && !existing.contains_key(&card.id)
        {
            records
                .entry(card.id.clone())
                .or_insert_with(|| DecisionRecord {
                    question: card.title.clone(),
                    ..Default::default()
                });
            offers.push(card);
        } else if row.is_prompt() && row.first && !offers.is_empty() {
            let reply = rows[ix + 1..].iter().take_while(|row| !row.first).fold(
                row.text.clone(),
                |mut text, row| {
                    text.push_str(&row.text);
                    text
                },
            );
            record_reply(&mut offers, &mut records, &reply);
        }
    }
    if let Some(reply) = queued_reply {
        record_reply(&mut offers, &mut records, reply);
    }
    records
}

impl TranscriptView {
    /// Roster restoration includes decisions that precede the loaded transcript tail.
    pub(crate) fn set_mono_decisions(
        &mut self,
        records: BTreeMap<String, DecisionRecord>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.mono_decisions.records == records {
            return false;
        }
        self.mono_decisions.cards.retain(|id, _| {
            records
                .get(id)
                .is_none_or(|record| record.answers.is_empty())
        });
        self.mono_decisions.records = records;
        self.refresh_legacy_mono_decisions();
        cx.notify();
        true
    }

    pub(super) fn refresh_legacy_mono_decisions(&mut self) {
        if self.mono {
            self.mono_decisions.legacy = legacy_records(
                &self.source,
                &self.mono_decisions.records,
                self.pending
                    .first()
                    .filter(|reply| reply.error.is_none())
                    .map(|reply| reply.text.as_ref()),
            );
        }
    }

    pub(crate) fn pending_mono_decisions(&self) -> BTreeMap<String, String> {
        if !self.mono {
            return BTreeMap::new();
        }
        self.mono_decisions
            .records
            .iter()
            .chain(self.mono_decisions.legacy.iter())
            .filter(|(_, record)| record.answers.is_empty())
            .map(|(id, record)| (id.clone(), record.question.clone()))
            .collect()
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
        if let Some(record) = self
            .mono_decisions
            .records
            .get(&id)
            .or_else(|| self.mono_decisions.legacy.get(&id))
            .filter(|r| !r.answers.is_empty())
        {
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
                            .child(format!("You chose: {}", record.answers.join(" · "))),
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
        let records = legacy_records(&rows, &BTreeMap::new(), Some("Skip"));
        assert!(records["first"].answers.is_empty());
        assert_eq!(records["second"].answers, ["Run tests"]);
        assert!(records["third"].answers.is_empty());
        assert_eq!(records["fourth"].answers, ["Skip"]);
    }
}
