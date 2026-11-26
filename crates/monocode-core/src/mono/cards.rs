use crate::agent::{Question, QuestionItem, QuestionOption};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

/// Native roster metadata. Answers and their user message are saved together.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionRecord {
    pub question: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<String>,
}

pub fn question(card: &Value) -> Option<Question> {
    if card["type"] != "choices" {
        return None;
    }
    let id = card["id"].as_str().filter(|id| !id.is_empty())?;
    let title = card["question"]
        .as_str()
        .unwrap_or("What should I do next?");
    let options = card["options"]
        .as_array()?
        .iter()
        .enumerate()
        .filter_map(|(ix, label)| {
            let label = label.as_str()?;
            Some(QuestionOption {
                id: ix.to_string(),
                label: label.to_owned(),
                description: card["optionDescriptions"][label]
                    .as_str()
                    .unwrap_or("")
                    .to_owned(),
            })
        })
        .collect::<Vec<_>>();
    (!options.is_empty()).then(|| Question {
        id: id.to_owned(),
        title: title.to_owned(),
        auto_resolve_at: None,
        items: vec![QuestionItem {
            id: id.to_owned(),
            prompt: title.to_owned(),
            multiple: true,
            allow_custom: true,
            options,
        }],
    })
}

pub fn decision_answers(answers: &[Value]) -> Result<Vec<String>, String> {
    let mut unique = HashSet::new();
    let answers = answers
        .iter()
        .map(|answer| {
            answer
                .as_str()
                .map(str::trim)
                .filter(|text| !text.is_empty() && text.chars().count() <= 4_000)
                .ok_or_else(|| "Provide an answer under 4000 characters".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|answer| unique.insert(*answer))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if answers.is_empty() {
        return Err("Choose an option or enter another answer".into());
    }
    Ok(answers)
}
fn text(v: &Value, key: &str, max: usize) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty() && s.chars().count() <= max)
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .ok_or_else(|| format!("Provide {key} under {max} characters"))
}
pub fn parse(v: &Value) -> Result<Value, String> {
    let kind = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or("Card type must be pr, session, choices or habit")?;
    let allowed = match kind {
        "pr" => vec!["repo", "number", "note"],
        "session" => vec!["sessionId", "note"],
        "choices" => vec!["options", "question"],
        "habit" => vec!["name", "instructions", "schedule"],
        _ => return Err("Card type must be pr, session, choices or habit".into()),
    };
    if v.as_object()
        .ok_or("Provide a card object")?
        .keys()
        .any(|k| k != "type" && !allowed.contains(&k.as_str()))
    {
        return Err(format!("Unexpected {kind} card field"));
    }
    let mut result = match kind {
        "pr" => {
            let number = v
                .get("number")
                .and_then(Value::as_u64)
                .filter(|n| *n > 0)
                .ok_or("Provide a positive PR number")?;
            let repo = v.get("repo").map(|_| text(v, "repo", 140)).transpose()?;
            if repo.as_ref().is_some_and(|r| {
                r.split('/').count() != 2
                    || !r
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c))
                    || r.starts_with('/')
                    || r.ends_with('/')
            }) {
                return Err("repo must be owner/name".into());
            }
            json!({"type":"pr","number":number,"repo":repo})
        }
        "session" => {
            let session = v
                .get("sessionId")
                .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
                .filter(|n| *n > 0)
                .ok_or("Provide sessionId")?;
            json!({"type":"session","sessionId":session})
        }
        "choices" => {
            let options = v
                .get("options")
                .and_then(Value::as_array)
                .filter(|options| !options.is_empty())
                .ok_or("Provide at least one option")?;
            let question = v
                .get("question")
                .map(|_| text(v, "question", 500))
                .transpose()?;
            let mut labels = Vec::with_capacity(options.len());
            let mut descriptions = BTreeMap::new();
            let mut seen = HashSet::new();
            for option in options {
                let (label, description) = if let Some(label) = option.as_str() {
                    (label, None)
                } else {
                    let option = option.as_object().ok_or("Provide an option label")?;
                    if option
                        .keys()
                        .any(|key| key != "label" && key != "description")
                    {
                        return Err("Unexpected choice option field".into());
                    }
                    let label = option
                        .get("label")
                        .and_then(Value::as_str)
                        .ok_or("Provide an option label")?;
                    let description = option
                        .get("description")
                        .map(|description| {
                            description
                                .as_str()
                                .filter(|s| s.chars().count() <= 400)
                                .ok_or("Option descriptions must be under 400 characters")
                        })
                        .transpose()?;
                    (label, description)
                };
                let label = label.trim();
                if label.is_empty() || label.chars().count() > 160 {
                    return Err("Options must contain 1–160 characters".into());
                }
                if !seen.insert(label) {
                    return Err("Option labels must be distinct".into());
                }
                labels.push(label);
                if let Some(description) = description {
                    descriptions.insert(label, description.trim());
                }
            }
            let mut card = json!({"type":"choices","options":labels});
            if let Some(question) = question {
                card["question"] = json!(question);
            }
            if !descriptions.is_empty() {
                card["optionDescriptions"] = json!(descriptions);
            }
            card
        }
        "habit" => {
            let schedule =
                super::habits::Schedule::from_cli(v.get("schedule").ok_or("Provide schedule")?)?;
            json!({"type":"habit","name":text(v,"name",80)?,"instructions":text(v,"instructions",4000)?,"schedule":schedule})
        }
        _ => unreachable!(),
    };
    if v.get("note").is_some() {
        result["note"] = json!(text(v, "note", 280)?);
    }
    Ok(result)
}
pub fn pull_request(cwd: &str, repo: Option<&str>, number: u64) -> Result<Value, String> {
    let binary = crate::scm::shell::resolve_gui_binary("gh").ok_or("GitHub CLI not found")?;
    let mut cmd = std::process::Command::new(binary);
    cmd.current_dir(cwd).args([
        "pr",
        "view",
        &number.to_string(),
        "--json",
        "title,state,url,isDraft,additions,deletions",
    ]);
    if let Some(repo) = repo {
        cmd.args(["--repo", repo]);
    }
    cmd.env("GH_PROMPT_DISABLED", "1");
    crate::scm::shell::apply_gui_env(&mut cmd);
    crate::process::configure_child(&mut cmd);
    let output = cmd.output().map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(1000)
            .collect());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cards_validate_fields_and_do_not_accept_arbitrary_links_or_commands() {
        assert!(parse(&json!({"type":"pr","repo":"owner/repo;echo x","number":1})).is_err());
        assert!(parse(&json!({"type":"choices","options":["a"],"command":"rm"})).is_err());
        assert!(parse(&json!({"type":"choices","options":["same"," same "]})).is_err());
        assert_eq!(
            parse(&json!({"type":"session","sessionId":"12"})).unwrap()["sessionId"],
            12
        );
        assert!(parse(&json!({"type":"habit","name":"Report","instructions":"Report builds","schedule":{"kind":"daily","time":"09:00"}})).is_ok());
    }
    #[test]
    fn decisions_support_more_than_four_options_and_custom_multiple_answers() {
        let mut card = parse(&json!({
            "type":"choices", "question":"Which checks run?",
            "options":["One","Two","Three","Four",{"label":"Five","description":"Slow"}]
        }))
        .unwrap();
        card["id"] = json!("first-card");
        let request = question(&card).unwrap();
        assert_eq!(request.id, "first-card");
        assert_eq!(request.title, "Which checks run?");
        assert_eq!(request.items[0].options.len(), 5);
        assert_eq!(request.items[0].options[4].description, "Slow");
        assert!(request.items[0].multiple && request.items[0].allow_custom);
        assert_eq!(
            decision_answers(&[json!("Two"), json!("Five"), json!("My own check")]).unwrap(),
            vec!["Two", "Five", "My own check"]
        );
    }
}
