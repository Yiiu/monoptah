//! Keep delegated traffic under its real parent, rather than merging it into
//! the assistant's answer or allowing a child completion to finish the turn.
use super::*;
use crate::tool::tasks::{AgentRun, AgentStep, Panel, TaskList};

#[derive(Default)]
pub(super) struct Activities {
    main_thread: Option<String>,
    owners: HashMap<String, String>,
    pending: Vec<Value>,
    reported_states: HashMap<String, ToolState>,
    pub(super) background_calls: std::collections::HashSet<String>,
    pub(super) claude_background: std::collections::HashSet<String>,
    /// Every running Claude task in start order, as claude.ts's backgroundTasks.
    pub(super) claude_tasks: Vec<BackgroundTask>,
    /// Most recent inventory announced to the UI.
    pub(super) published: Vec<BackgroundTask>,
}
fn text(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| !s.trim().is_empty())
}
fn state(status: &str) -> Option<ToolState> {
    match status.to_lowercase().as_str() {
        "completed" | "complete" | "done" | "finished" | "closed" | "stopped" => {
            Some(ToolState::Completed)
        }
        "failed" | "error" | "errored" | "notfound" | "not_found" => Some(ToolState::Failed),
        "interrupted" | "cancelled" | "canceled" | "killed" | "aborted" => {
            Some(ToolState::Cancelled)
        }
        "running" | "inprogress" | "in_progress" | "pending" => Some(ToolState::Running),
        _ => None,
    }
}
impl Protocol {
    pub(super) fn activity(&mut self, value: &Value, events: &mut Vec<Event>) -> bool {
        match self.request.kind {
            Kind::Codex => self.codex_activity(value, events),
            Kind::Claude => self.claude_activity(value, events),
            _ => false,
        }
    }
    fn agent_step(
        &mut self,
        parent: &str,
        step: AgentStep,
        model: Option<&str>,
        events: &mut Vec<Event>,
    ) {
        let Some(tool) = self.tools.get_mut(parent) else {
            return;
        };
        let Some(run) = tool.agent_mut() else { return };
        if let Some(model) = model {
            run.model = Some(model.into());
        }
        run.upsert(step);
        events.push(Event::Tool(tool.clone()));
    }
    fn codex_activity(&mut self, value: &Value, events: &mut Vec<Event>) -> bool {
        if value["id"] == 2 {
            if let Some(id) = text(&value["result"]["thread"]["id"]) {
                self.activities.main_thread = Some(id.into());
            }
        }
        let p = &value["params"];
        let method = value["method"].as_str().unwrap_or("");
        // Approval/question requests still go through the shared approval UI.
        if value.get("id").is_some()
            && (method.ends_with("/requestApproval") || method == "item/tool/requestUserInput")
        {
            return false;
        }
        let thread = text(&p["threadId"])
            .or_else(|| text(&p["thread_id"]))
            .or_else(|| text(&p["thread"]["id"]));
        if let Some(parent) = thread
            .and_then(|id| self.activities.owners.get(id))
            .cloned()
        {
            self.codex_child(&parent, method, p, events);
            return true;
        }
        if let (Some(thread), Some(main)) = (thread, self.activities.main_thread.as_deref()) {
            if thread != main {
                // A child can speak before its spawn snapshot identifies it.
                if matches!(
                    method,
                    "item/started"
                        | "item/completed"
                        | "turn/completed"
                        | "turn/aborted"
                        | "thread/started"
                ) && self.activities.pending.len() < 64
                    && value.to_string().len() <= 16 * 1024
                {
                    self.activities.pending.push(value.clone());
                }
                return true;
            }
        }
        if method == "turn/completed" {
            for item in p["turn"]["items"].as_array().into_iter().flatten() {
                if matches!(
                    item["type"].as_str(),
                    Some("collabAgentToolCall" | "subAgentActivity")
                ) {
                    self.codex_activity(&json!({"method":"item/completed","params":{"threadId":thread,"item":item}}), events);
                }
            }
        }
        if method == "turn/plan/updated" {
            let Some(list) = TaskList::from_value(p) else {
                return true;
            };
            let Some(turn) = text(&p["turnId"]).or(self.active_turn.as_deref()) else {
                return true;
            };
            let id = format!("plan:{turn}");
            let tool = self
                .tools
                .entry(id.clone())
                .or_insert_with(|| ToolCall::new(&id, "update_plan"));
            tool.input = p.to_string();
            tool.panel = Some(Panel::Tasks(list));
            tool.state = ToolState::Completed; // A received snapshot, not a claim its tasks finished.
            events.push(Event::Tool(tool.clone()));
            return true;
        }
        if !matches!(method, "item/started" | "item/completed") {
            return false;
        }
        let item = &p["item"];
        if !matches!(
            item["type"].as_str(),
            Some("collabAgentToolCall" | "subAgentActivity")
        ) {
            return false;
        }
        let Some(id) = text(&item["id"]) else {
            return true;
        };
        let action = item["tool"].as_str().unwrap_or("");
        let mut ids: Vec<String> = ["agentThreadId", "agent_thread_id"]
            .iter()
            .filter_map(|key| text(&item[*key]).map(str::to_owned))
            .collect();
        for receiver in item
            .get("receiverThreadIds")
            .or_else(|| item.get("receiver_thread_ids"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(receiver) = text(receiver) {
                ids.push(receiver.into());
            }
        }
        let states = item
            .get("agentsStates")
            .or_else(|| item.get("agents_states"))
            .and_then(Value::as_object);
        if let Some(states) = states {
            ids.extend(states.keys().cloned());
        }
        ids.sort();
        ids.dedup();
        let spawn =
            matches!(action, "spawnAgent" | "resumeAgent") || item["type"] == "subAgentActivity";
        if spawn {
            let existing = ids
                .iter()
                .find_map(|id| self.activities.owners.get(id))
                .cloned();
            let owner = existing.as_deref().unwrap_or(id);
            let tool = self
                .tools
                .entry(owner.into())
                .or_insert_with(|| ToolCall::new(owner, "collabAgentToolCall"));
            if tool.panel.is_none() {
                tool.panel = Some(Panel::Agent(AgentRun::default()));
            }
            tool.input = item.to_string();
            if let Some(run) = tool.agent_mut() {
                for id in &ids {
                    if !run.ids.contains(id) {
                        run.ids.push(id.clone());
                    }
                    self.activities
                        .owners
                        .entry(id.clone())
                        .or_insert_with(|| owner.into());
                }
                // A second item may describe an already-named child. Keep the
                // original task title, but let it fill a missing path label.
                if owner == id || (run.name.is_none() && run.brief.is_empty()) {
                    if let Some(name) = AgentRun::from_input(item).name {
                        run.name = Some(name);
                    }
                }
                if let Some(model) = text(&item["model"]) {
                    run.model = Some(model.into());
                }
                if let Some(brief) = text(&item["prompt"]).or_else(|| text(&item["description"])) {
                    run.brief = brief.into();
                }
            }
            // The spawn RPC returning does not mean the agent has completed.
            if item["status"] == "failed" || item["kind"] == "interrupted" {
                tool.state = ToolState::Failed;
                if let Some(message) =
                    text(&item["error"]["message"]).or_else(|| text(&item["message"]))
                {
                    tool.output = message.into();
                    tool.bound_output();
                }
            }
            events.push(Event::Tool(tool.clone()));
        }
        if let Some(states) = states {
            for (thread, reported) in states {
                if let Some(status) = reported["status"].as_str().and_then(state) {
                    self.settle_child(thread, status, text(&reported["message"]), events);
                }
            }
        }
        let pending = std::mem::take(&mut self.activities.pending);
        for frame in pending {
            let thread =
                text(&frame["params"]["threadId"]).or_else(|| text(&frame["params"]["thread_id"]));
            if let Some(owner) = thread
                .and_then(|id| self.activities.owners.get(id))
                .cloned()
            {
                self.codex_child(
                    &owner,
                    frame["method"].as_str().unwrap_or(""),
                    &frame["params"],
                    events,
                );
            } else {
                self.activities.pending.push(frame);
            }
        }
        true
    }
    fn codex_child(&mut self, parent: &str, method: &str, params: &Value, events: &mut Vec<Event>) {
        if method == "thread/started" {
            if let Some(model) = text(&params["thread"]["model"]) {
                if let Some(tool) = self.tools.get_mut(parent) {
                    if let Some(run) = tool.agent_mut() {
                        run.model = Some(model.into());
                    }
                    events.push(Event::Tool(tool.clone()));
                }
            }
        } else if matches!(method, "item/started" | "item/completed") {
            let item = &params["item"];
            let Some(id) = text(&item["id"]) else { return };
            let kind = item["type"].as_str().unwrap_or("");
            if matches!(kind, "agentMessage" | "reasoning") {
                if method != "item/completed" {
                    return;
                }
                let narration = text(&item["text"]).map(str::to_owned).unwrap_or_else(|| {
                    item["summary"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|part| text(&part["text"]))
                        .collect::<Vec<_>>()
                        .join("\n")
                });
                if !narration.is_empty() {
                    self.agent_step(
                        parent,
                        AgentStep {
                            id: id.into(),
                            kind: if kind == "reasoning" {
                                "reasoning"
                            } else {
                                "message"
                            }
                            .into(),
                            text: narration,
                            state: None,
                            detail: String::new(),
                            ..AgentStep::default()
                        },
                        None,
                        events,
                    );
                }
            } else {
                let mut child_events = Vec::new();
                self.codex_tool(item, method == "item/completed", &mut child_events);
                for child in child_events {
                    if let Event::Tool(child) = child {
                        self.agent_step(parent, AgentStep::from_tool_call(&child), None, events);
                    }
                }
            }
        } else if matches!(method, "turn/completed" | "turn/aborted") {
            if let (Some(thread), Some(status)) = (
                text(&params["threadId"]).or_else(|| text(&params["thread_id"])),
                params["turn"]["status"]
                    .as_str()
                    .and_then(state)
                    .or_else(|| (method == "turn/aborted").then_some(ToolState::Cancelled)),
            ) {
                self.settle_child(
                    thread,
                    status,
                    text(&params["turn"]["error"]["message"]),
                    events,
                );
            }
        }
    }
    fn settle_child(
        &mut self,
        thread: &str,
        status: ToolState,
        message: Option<&str>,
        events: &mut Vec<Event>,
    ) {
        self.activities
            .reported_states
            .insert(thread.into(), status);
        let Some(owner) = self.activities.owners.get(thread) else {
            return;
        };
        let Some(tool) = self.tools.get_mut(owner) else {
            return;
        };
        if let Some(run) = tool.agent_mut() {
            let states = &self.activities.reported_states;
            tool.state = if run
                .ids
                .iter()
                .any(|id| states.get(id) == Some(&ToolState::Failed))
            {
                ToolState::Failed
            } else if !run.ids.is_empty()
                && run.ids.iter().all(|id| {
                    matches!(
                        states.get(id),
                        Some(ToolState::Completed | ToolState::Cancelled)
                    )
                })
                && run
                    .ids
                    .iter()
                    .any(|id| states.get(id) == Some(&ToolState::Cancelled))
            {
                ToolState::Cancelled
            } else if !run.ids.is_empty()
                && run
                    .ids
                    .iter()
                    .all(|id| states.get(id) == Some(&ToolState::Completed))
            {
                ToolState::Completed
            } else {
                ToolState::Running
            };
        }
        if let Some(message) = message {
            tool.output = message.into();
            tool.bound_output();
        }
        events.push(Event::Tool(tool.clone()));
    }
    fn claude_activity(&mut self, value: &Value, events: &mut Vec<Event>) -> bool {
        self.claude_background_list(value);
        let parent = text(&value["parent_tool_use_id"]);
        if let Some(parent) = parent {
            match value["type"].as_str() {
                Some("assistant") => {
                    let model = text(&value["message"]["model"]);
                    let message_id = text(&value["message"]["id"]);
                    for (index, block) in value["message"]["content"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .enumerate()
                    {
                        match block["type"].as_str() {
                            Some("tool_use") => {
                                let Some(id) = text(&block["id"]) else {
                                    continue;
                                };
                                let mut child =
                                    ToolCall::new(id, block["name"].as_str().unwrap_or("Tool"));
                                child.input = block["input"].to_string();
                                self.agent_step(
                                    parent,
                                    AgentStep::from_tool_call(&child),
                                    model,
                                    events,
                                );
                            }
                            Some("text" | "thinking") => {
                                let Some(message_id) = message_id else {
                                    continue;
                                };
                                let thinking = block["type"] == "thinking";
                                let narration = block[if thinking { "thinking" } else { "text" }]
                                    .as_str()
                                    .unwrap_or("");
                                if narration.is_empty() {
                                    continue;
                                }
                                self.agent_step(
                                    parent,
                                    AgentStep {
                                        id: format!("{message_id}:{index}"),
                                        kind: if thinking { "reasoning" } else { "message" }.into(),
                                        text: narration.into(),
                                        state: None,
                                        detail: String::new(),
                                        ..AgentStep::default()
                                    },
                                    model,
                                    events,
                                );
                            }
                            _ => {}
                        }
                    }
                }
                Some("user") => {
                    for block in value["message"]["content"].as_array().into_iter().flatten() {
                        if block["type"] != "tool_result" {
                            continue;
                        }
                        let Some(id) = text(&block["tool_use_id"]) else {
                            continue;
                        };
                        let Some(tool) = self.tools.get_mut(parent) else {
                            continue;
                        };
                        let Some(run) = tool.agent_mut() else {
                            continue;
                        };
                        let Some(step) = run.steps.iter_mut().find(|step| step.id == id) else {
                            continue;
                        };
                        step.state = Some(if block["is_error"] == true {
                            ToolState::Failed
                        } else {
                            ToolState::Completed
                        });
                        step.detail = text_content(&block["content"]);
                        crate::tool::tasks::bound(&mut step.detail, 16 * 1024);
                        events.push(Event::Tool(tool.clone()));
                    }
                }
                Some("tool_progress") => self.claude_progress(value, parent, events),
                _ => {}
            }
            // Including nested stream deltas: the completed assistant snapshot
            // supplies stable message IDs and avoids duplicate streamed narration.
            return true;
        }
        if value["type"] == "system"
            && matches!(
                value["subtype"].as_str(),
                Some("task_started" | "task_progress" | "task_updated" | "task_notification")
            )
        {
            let subtype = value["subtype"].as_str().unwrap_or("");
            let patch = if subtype == "task_updated" {
                &value["patch"]
            } else {
                value
            };
            let status = patch["status"].as_str().and_then(state);
            let terminal = subtype == "task_notification"
                || status
                    .as_ref()
                    .is_some_and(|status| *status != ToolState::Running);
            if let Some(id) = text(&value["task_id"]) {
                if value["ambient"] == true || patch["ambient"] == true {
                    self.remove_claude_task(id);
                    return true;
                }
                if terminal {
                    let kind = self.remove_claude_task(id);
                    if self.claude_result_seen {
                        let deadline = Instant::now() + CLAUDE_CONTINUATION_GRACE;
                        match kind {
                            Some(TaskKind::Shell) => self.claude_drain_at = Some(deadline),
                            Some(_) => self.claude_task_wake_at = Some(deadline),
                            None => {}
                        }
                    }
                } else {
                    if let Some(backgrounded) = patch["is_backgrounded"].as_bool() {
                        if backgrounded {
                            self.activities.claude_background.insert(id.into());
                        } else {
                            self.activities.claude_background.remove(id);
                        }
                    }
                    if matches!(subtype, "task_started" | "task_updated") {
                        self.upsert_claude_task(id, value, patch, subtype == "task_started");
                    }
                }
            } else if value["ambient"] == true || patch["ambient"] == true {
                return true;
            }
            let owner = text(&value["tool_use_id"]).map(str::to_owned).or_else(|| {
                text(&value["task_id"]).and_then(|id| self.activities.owners.get(id).cloned())
            });
            if let Some(tool) = owner.as_deref().and_then(|owner| self.tools.get_mut(owner)) {
                if tool.agent_mut().is_some() {
                    if let Some(id) = text(&value["task_id"]) {
                        if let Some(run) = tool.agent_mut() {
                            if !run.ids.iter().any(|old| old == id) {
                                run.ids.push(id.into());
                            }
                        }
                        self.activities.owners.insert(id.into(), tool.id.clone());
                    }
                    if terminal || patch["is_backgrounded"] == false {
                        self.activities.background_calls.remove(&tool.id);
                    } else if patch["is_backgrounded"] == true {
                        self.activities.background_calls.insert(tool.id.clone());
                    }
                    if let Some(status) = status {
                        tool.state = status;
                    } else if subtype == "task_started" {
                        tool.state = ToolState::Running;
                    }
                    if let Some(run) = tool.agent_mut() {
                        // Match React: keep the run's name once its trail starts.
                        if run.steps.is_empty() {
                            if let Some(description) = text(&patch["description"]) {
                                run.brief = description.into();
                            }
                        }
                        if let Some(summary) =
                            text(&value["summary"]).filter(|_| subtype == "task_progress")
                        {
                            if let Some(id) = text(&value["task_id"]) {
                                run.upsert(AgentStep {
                                    id: format!("task:{id}"),
                                    kind: "message".into(),
                                    text: summary.into(),
                                    state: None,
                                    detail: String::new(),
                                    ..AgentStep::default()
                                });
                            }
                        }
                    }
                    if subtype == "task_notification" {
                        if let Some(summary) = text(&value["summary"]) {
                            tool.output = summary.into();
                            tool.bound_output();
                        }
                    } else if let Some(error) = text(&patch["error"]) {
                        tool.output = error.into();
                        tool.bound_output();
                    }
                    events.push(Event::Tool(tool.clone()));
                }
            }
            return true;
        }
        false
    }
    fn upsert_claude_task(&mut self, id: &str, value: &Value, patch: &Value, insert: bool) {
        let description = text(&patch["description"]);
        let kind = text(&patch["task_type"])
            .or_else(|| text(&value["task_type"]))
            .map(classify_task);
        let source = text(&value["tool_use_id"]).and_then(|id| self.tools.get(id));
        let command = text(&patch["command"]).map(str::to_owned).or_else(|| {
            source
                .filter(|tool| tool.name == "Bash")
                .and_then(|tool| serde_json::from_str::<Value>(&tool.input).ok())
                .and_then(|input| text(&input["command"]).map(str::to_owned))
        });
        let tasks = &mut self.activities.claude_tasks;
        if let Some(task) = tasks.iter_mut().find(|task| task.id == id) {
            if let Some(description) = description {
                task.description = description.into();
            }
            if let Some(kind) = kind {
                task.kind = kind;
                task.agent = kind == TaskKind::Agent;
            }
            if command.is_some() {
                task.command = command;
            }
        } else if insert {
            let kind = kind.unwrap_or_else(|| match source.map(|tool| tool.name.as_str()) {
                Some("Bash") => TaskKind::Shell,
                Some("Task" | "Agent") => TaskKind::Agent,
                _ => TaskKind::Other,
            });
            tasks.push(BackgroundTask {
                id: id.into(),
                description: description
                    .unwrap_or(if kind == TaskKind::Shell {
                        "Background command"
                    } else {
                        "Background task"
                    })
                    .into(),
                kind,
                agent: kind == TaskKind::Agent,
                command,
                started_at: crate::time::now_ms(),
            });
        }
    }
    fn remove_claude_task(&mut self, id: &str) -> Option<TaskKind> {
        self.activities.claude_background.remove(id);
        if let Some(owner) = self.activities.owners.get(id) {
            self.activities.background_calls.remove(owner);
        }
        let tasks = &mut self.activities.claude_tasks;
        let index = tasks.iter().position(|task| task.id == id)?;
        Some(tasks.remove(index).kind)
    }

    /// Claude's full list of running tasks replaces the tracked one.
    fn claude_background_list(&mut self, value: &Value) {
        if value["type"] != "system" || value["subtype"] != "background_tasks_changed" {
            return;
        }
        let Some(rows) = value["tasks"].as_array() else {
            return;
        };
        let rows: Vec<_> = rows
            .iter()
            .filter(|row| row["ambient"] != true)
            .filter_map(|row| Some((text(&row["task_id"])?, row)))
            .collect();
        let missing: Vec<_> = self
            .activities
            .claude_tasks
            .iter()
            .filter(|task| !rows.iter().any(|(id, _)| *id == task.id))
            .map(|task| task.id.clone())
            .collect();
        if !missing.is_empty() && self.claude_result_seen {
            // An inventory removal does not promise Claude will resume. Keep
            // the wire briefly so a real continuation can still arrive.
            self.claude_drain_at = Some(Instant::now() + CLAUDE_CONTINUATION_GRACE);
        }
        for id in missing {
            self.remove_claude_task(&id);
        }
        self.activities.claude_background = rows.iter().map(|(id, _)| (*id).into()).collect();
        for (id, row) in rows {
            self.upsert_claude_task(id, row, row, true);
        }
    }
    /// Shells stay visible across assistant results and continuations.
    pub(super) fn sync_background(&mut self, events: &mut Vec<Event>) {
        let waiting = self.activities.claude_tasks.iter().filter(|task| {
            self.claude_result_seen
                || (task.kind == TaskKind::Shell
                    && self.activities.claude_background.contains(&task.id))
        });
        if waiting.clone().ne(self.activities.published.iter()) {
            let waiting: Vec<_> = waiting.cloned().collect();
            self.activities.published = waiting.clone();
            events.push(Event::Background(waiting));
        }
    }
    fn claude_progress(&mut self, value: &Value, parent: &str, events: &mut Vec<Event>) {
        let (Some(id), Some(name)) = (text(&value["tool_use_id"]), text(&value["tool_name"]))
        else {
            return;
        };
        // Progress is a child's step, never the parent Agent's final report.
        self.agent_step(
            parent,
            AgentStep {
                id: id.into(),
                kind: "tool".into(),
                text: name.into(),
                state: Some(ToolState::Running),
                detail: String::new(),
                tool_kind: Some(name.into()),
                ..AgentStep::default()
            },
            None,
            events,
        );
    }
}
fn classify_task(task_type: &str) -> TaskKind {
    let name = task_type.to_ascii_lowercase();
    match name.as_str() {
        "local_bash" | "remote_bash" | "bash" | "shell" => TaskKind::Shell,
        "local_agent" | "remote_agent" => TaskKind::Agent,
        _ if name.contains("monitor") => TaskKind::Monitor,
        _ if name.contains("workflow") => TaskKind::Workflow,
        _ => TaskKind::Other,
    }
}
fn text_content(value: &Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn protocol(kind: Kind) -> Protocol {
        Protocol::new(Request {
            kind,
            binary: "unused".into(),
            cwd: "/tmp".into(),
            prompt: "Hello".into(),
            provider_id: None,
            model: None,
            settings: Default::default(),
        })
    }
    fn tool(events: &[Event]) -> &ToolCall {
        events
            .iter()
            .rev()
            .find_map(|e| {
                if let Event::Tool(tool) = e {
                    Some(tool)
                } else {
                    None
                }
            })
            .unwrap()
    }
    fn spawn(p: &mut Protocol) {
        p.frame(json!({"method":"item/completed","params":{"threadId":"main","item":{"id":"spawn-id","type":"collabAgentToolCall","tool":"spawnAgent","status":"completed","receiverThreadIds":["child"],"prompt":"Audit correctness","model":"child-model"}}}), &mut Vec::new()).unwrap();
    }
    #[test]
    fn codex_path_titles_match_tauri_for_both_item_formats() {
        for kind in ["collabAgentToolCall", "subAgentActivity"] {
            for (key, path, expected) in [
                ("agentPath", "/root/explore-auth", "Explore Auth subagent"),
                (
                    "agent_path",
                    "\\root\\pairing__dialog-parity\\",
                    "Pairing Dialog Parity subagent",
                ),
            ] {
                let mut p = protocol(Kind::Codex);
                let mut item = json!({
                    "id":"spawn-id", "type":kind, "tool":"spawnAgent", "kind":"started",
                    "receiverThreadIds":["child"], "agentThreadId":"child",
                    "prompt":"Full instructions for the delegated task.", "model":"child-model"
                });
                item[key] = json!(path);
                let events = p
                    .frame(
                        json!({"method":"item/completed","params":{"threadId":"main","item":item}}),
                        &mut Vec::new(),
                    )
                    .unwrap();
                let Some(Panel::Agent(run)) = tool(&events).panel() else {
                    panic!()
                };
                assert_eq!(run.display_name(), expected);
                assert_eq!(run.display_brief(), expected);
                assert_eq!(run.model.as_deref(), Some("child-model"));
                assert_eq!(tool(&events).state, ToolState::Running);

                // Sparse completion and child work must retain the title.
                let events = p
                    .frame(
                        json!({"method":"item/completed","params":{"threadId":"main","item":{
                            "id":"spawn-id", "type":kind, "tool":"spawnAgent", "kind":"interacted",
                            "receiverThreadIds":["child"], "agentThreadId":"child"
                        }}}),
                        &mut Vec::new(),
                    )
                    .unwrap();
                let Some(Panel::Agent(run)) = tool(&events).panel() else {
                    panic!()
                };
                assert_eq!(run.display_name(), expected);
                let events = p.frame(
                    json!({"method":"item/completed","params":{"threadId":"child","item":{
                        "id":"read", "type":"commandExecution", "command":"cat source.rs", "status":"completed"
                    }}}), &mut Vec::new(),
                ).unwrap();
                let Some(Panel::Agent(run)) = tool(&events).panel() else {
                    panic!()
                };
                assert_eq!(run.display_name(), expected);
                assert_eq!(run.steps.len(), 1);
                // Desktop persistence carries the same label back to native.
                let restored = crate::tool::tasks::from_desktop(&json!({
                    "role":"tool", "agentRun":crate::tool::tasks::desktop_agent(&run)
                }))
                .unwrap();
                assert_eq!(restored, Panel::Agent(run));
            }
        }
    }
    #[test]
    fn codex_duplicate_path_items_keep_the_original_task_title() {
        let mut p = protocol(Kind::Codex);
        spawn(&mut p);
        let events = p
            .frame(
                json!({"method":"item/completed","params":{"threadId":"main","item":{
                    "id":"activity-id", "type":"subAgentActivity", "kind":"started",
                    "agentPath":"/root/explore-auth", "agentThreadId":"child"
                }}}),
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(tool(&events).id, "spawn-id");
        let Some(Panel::Agent(run)) = tool(&events).panel() else {
            panic!()
        };
        assert_eq!(run.display_name(), "Audit correctness");
        assert!(!p.tools.contains_key("activity-id"));
    }
    #[test]
    fn codex_completed_spawn_wait_and_child_lifecycle_keep_actual_owner() {
        let mut p = protocol(Kind::Codex);
        spawn(&mut p);
        assert_eq!(p.tools["spawn-id"].state, ToolState::Running);
        p.active_turn = Some("parent-turn".into());
        let child = p.frame(json!({"method":"item/completed","params":{"threadId":"child","item":{"id":"child-answer","type":"agentMessage","text":"Only the child said this"}}}), &mut Vec::new()).unwrap();
        assert_eq!(child.len(), 1);
        assert_eq!(tool(&child).id, "spawn-id");
        let Some(Panel::Agent(run)) = tool(&child).panel() else {
            panic!()
        };
        assert_eq!(run.ids, vec!["child"]);
        assert_eq!(run.display_name(), "Audit correctness");
        assert_eq!(run.model.as_deref(), Some("child-model"));
        assert_eq!(run.steps[0].id, "child-answer");
        assert_eq!(run.steps[0].text, "Only the child said this");
        let completed = p.frame(json!({"method":"turn/completed","params":{"threadId":"child","turn":{"id":"child-turn","status":"completed"}}}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&completed).state, ToolState::Completed);
        assert_eq!(p.active_turn.as_deref(), Some("parent-turn"));
        assert!(!p.done);
        let settled = p.frame(json!({"method":"item/completed","params":{"threadId":"main","item":{"id":"wait-id","type":"collabAgentToolCall","tool":"wait","agentsStates":{"child":{"status":"failed","message":"Provider failure"}}}}}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&settled).id, "spawn-id");
        assert_eq!(tool(&settled).state, ToolState::Failed);
        assert_eq!(tool(&settled).output, "Provider failure");
        assert!(!p.tools.contains_key("wait-id"));
    }
    #[test]
    fn codex_early_child_snapshots_replay_and_do_not_finish_main_turn() {
        let mut p = protocol(Kind::Codex);
        p.activities.main_thread = Some("main".into());
        let events = p.frame(json!({"method":"item/completed","params":{"threadId":"child","item":{"id":"early","type":"agentMessage","text":"Early snapshot"}}}), &mut Vec::new()).unwrap();
        assert!(events.is_empty());
        spawn(&mut p);
        let Some(Panel::Agent(run)) = p.tools["spawn-id"].panel() else {
            panic!()
        };
        assert_eq!(run.steps[0].id, "early");
        assert!(p.activities.pending.is_empty());
        let approval = p.frame(json!({"id":88,"method":"item/commandExecution/requestApproval","params":{"threadId":"child","command":"write"}}), &mut Vec::new()).unwrap();
        assert!(approval
            .iter()
            .any(|event| matches!(event, Event::Approval(_))));
    }
    #[test]
    fn codex_plan_snapshots_update_one_real_turn_and_clear_explicitly() {
        let mut p = protocol(Kind::Codex);
        let a = p.frame(json!({"method":"turn/plan/updated","params":{"turnId":"real-turn","explanation":"Inspect first","plan":[{"step":"Inspect","status":"in_progress"}]}}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&a).id, "plan:real-turn");
        let b = p
            .frame(
                json!({"method":"turn/plan/updated","params":{"turnId":"real-turn","plan":[]}}),
                &mut Vec::new(),
            )
            .unwrap();
        assert_eq!(tool(&a).id, tool(&b).id);
        let Some(Panel::Tasks(list)) = tool(&b).panel() else {
            panic!()
        };
        assert!(list.items.is_empty());
    }
    #[test]
    fn claude_child_work_never_leaks_into_parent_answer_or_metrics() {
        let mut p = protocol(Kind::Claude);
        p.frame(json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"agent-call","name":"Agent","input":{"description":"Correctness review","model":"requested"}}]}}), &mut Vec::new()).unwrap();
        let child = json!({"type":"assistant","parent_tool_use_id":"agent-call","message":{"id":"child-message","model":"actual-child-model","usage":{"input_tokens":999999},"content":[{"type":"text","text":"Child narration"},{"type":"tool_use","id":"child-tool","name":"Read","input":{"file_path":"source.rs"}}]}});
        let events = p.frame(child.clone(), &mut Vec::new()).unwrap();
        assert!(events.iter().all(|event| matches!(event, Event::Tool(_))));
        let Some(Panel::Agent(run)) = tool(&events).panel() else {
            panic!()
        };
        assert_eq!(run.model.as_deref(), Some("actual-child-model"));
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[0].id, "child-message:0");
        p.frame(json!({"type":"user","parent_tool_use_id":"agent-call","message":{"content":[{"type":"tool_result","tool_use_id":"child-tool","content":"Read result"}]}}), &mut Vec::new()).unwrap();
        p.frame(child, &mut Vec::new()).unwrap();
        let Some(Panel::Agent(run)) = p.tools["agent-call"].panel() else {
            panic!()
        };
        assert_eq!(run.steps.len(), 2);
        assert_eq!(run.steps[1].state, Some(ToolState::Completed));
        assert_eq!(run.steps[1].detail, "Read result");
        assert!(p.tools["agent-call"].output.is_empty());
    }
    #[test]
    fn claude_task_descriptions_keep_the_name_after_child_work_starts() {
        let mut p = protocol(Kind::Claude);
        p.frame(
            json!({"type":"assistant","message":{"content":[{
                "type":"tool_use","id":"agent-call","name":"Agent",
                "input":{"description":"Review implementation"}
            }]}}),
            &mut Vec::new(),
        )
        .unwrap();
        // Task metadata can refine the name before the child starts working.
        p.frame(
            json!({"type":"system","subtype":"task_started","task_id":"review-task",
                "tool_use_id":"agent-call","description":"Correctness review"}),
            &mut Vec::new(),
        )
        .unwrap();
        p.frame(
            json!({"type":"assistant","parent_tool_use_id":"agent-call",
            "message":{"id":"child-message","content":[{
                "type":"tool_use","id":"child-read","name":"Read",
                "input":{"file_path":"source.rs"}
            }]}}),
            &mut Vec::new(),
        )
        .unwrap();

        for update in [
            json!({"type":"system","subtype":"task_progress","task_id":"review-task",
                "description":"Running cd /repo && cargo check",
                "summary":"Inspecting files"}),
            json!({"type":"system","subtype":"task_updated","task_id":"review-task",
                "patch":{"description":"Running Read phases."}}),
            json!({"type":"system","subtype":"task_notification","task_id":"review-task",
                "description":"Finished reviewing source.rs","status":"completed",
                "summary":"No regressions found."}),
        ] {
            let events = p.frame(update, &mut Vec::new()).unwrap();
            let call = tool(&events);
            let Some(Panel::Agent(run)) = call.panel() else {
                panic!()
            };
            assert_eq!(run.display_name(), "Correctness review");
            assert_eq!(run.display_brief(), "Correctness review");
            assert_eq!(run.status_line(&call.state), "1 step");
            assert!(run.steps.iter().any(|step| step.text == "Inspecting files"));
        }
        assert_eq!(p.tools["agent-call"].state, ToolState::Completed);
        assert_eq!(p.tools["agent-call"].output, "No regressions found.");
    }
    #[test]
    fn claude_task_ids_progress_and_reports_come_from_task_events() {
        let mut p = protocol(Kind::Claude);
        p.tools
            .insert("agent".into(), ToolCall::new("agent", "Agent"));
        p.frame(json!({"type":"system","subtype":"task_started","task_id":"actual-task","tool_use_id":"agent","description":"Review","is_backgrounded":true}), &mut Vec::new()).unwrap();
        p.frame(json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"agent","content":"Background launch receipt"}]}}), &mut Vec::new()).unwrap();
        assert_eq!(p.tools["agent"].state, ToolState::Running);
        assert!(p.tools["agent"].output.is_empty());
        let progress = p.frame(json!({"type":"system","subtype":"task_progress","task_id":"actual-task","summary":"Inspecting files"}), &mut Vec::new()).unwrap();
        let Some(Panel::Agent(run)) = tool(&progress).panel() else {
            panic!()
        };
        assert_eq!(run.ids, vec!["actual-task"]);
        assert_eq!(run.steps[0].text, "Inspecting files");
        assert!(tool(&progress).output.is_empty());
        let done = p.frame(json!({"type":"system","subtype":"task_notification","task_id":"actual-task","status":"failed","summary":"Provider reported failure"}), &mut Vec::new()).unwrap();
        assert_eq!(tool(&done).state, ToolState::Failed);
        assert_eq!(tool(&done).output, "Provider reported failure");
        let orphan = p.frame(json!({"type":"assistant","parent_tool_use_id":"unknown","message":{"id":"x","content":[{"type":"text","text":"Unattributed child"}]}}), &mut Vec::new()).unwrap();
        assert!(orphan.is_empty());
    }
    #[test]
    fn claude_reports_shells_during_the_answer_and_subagents_once_it_yields() {
        let mut p = protocol(Kind::Claude);
        let frame = |p: &mut Protocol, value: Value| p.frame(value, &mut Vec::new()).unwrap();
        let inventory = |events: Vec<Event>| {
            events.into_iter().find_map(|event| match event {
                Event::Background(tasks) => Some(tasks),
                _ => None,
            })
        };
        frame(
            &mut p,
            json!({"type":"assistant","message":{"content":[{"type":"tool_use","id":"dev-call","name":"Bash","input":{"command":"npm run dev","run_in_background":true}}]}}),
        );
        let shells = inventory(frame(&mut p, json!({"type":"system","subtype":"task_started","task_id":"dev","tool_use_id":"dev-call","description":"Start dev server","task_type":"local_bash","is_backgrounded":true}))).unwrap();
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].command.as_deref(), Some("npm run dev"));
        // Work the agent is still doing in the foreground is not background.
        assert!(inventory(frame(&mut p, json!({"type":"system","subtype":"task_started","task_id":"review","description":"Review","task_type":"local_agent"}))).is_none());

        let yielded = frame(&mut p, json!({"type":"result","result":"Started it."}));
        assert!(
            yielded.contains(&Event::TurnActivity(TurnActivity::Yielded {
                wake_tasks: 1
            }))
        );
        let tasks = inventory(yielded).unwrap();
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].description, "Start dev server");
        assert_eq!(tasks[0].command.as_deref(), Some("npm run dev"));
        assert_eq!(tasks[0].kind, TaskKind::Shell);
        assert_eq!(tasks[1].kind, TaskKind::Agent);
        assert!(tasks[1].agent);
        assert!(inventory(p.lifecycle_tick(Instant::now(), &mut Vec::new()).unwrap()).is_none());

        let tasks = inventory(frame(&mut p, json!({"type":"system","subtype":"task_updated","task_id":"review","patch":{"status":"completed"}}))).unwrap();
        assert_eq!(
            tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["dev"]
        );

        let empty = frame(
            &mut p,
            json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}),
        );
        assert_eq!(inventory(empty), Some(Vec::new()));
    }

    #[test]
    fn claude_does_not_wait_for_a_task_missing_from_the_full_inventory() {
        let mut p = protocol(Kind::Claude);
        let mut stdin = Vec::new();
        p.frame(
            json!({"type":"system","subtype":"task_started","task_id":"review",
            "task_type":"local_agent","is_backgrounded":true}),
            &mut stdin,
        )
        .unwrap();
        let initial = p
            .frame(
                json!({"type":"result","result":"I delegated the review."}),
                &mut stdin,
            )
            .unwrap();
        assert!(
            initial.contains(&Event::TurnActivity(TurnActivity::Yielded {
                wake_tasks: 1
            }))
        );

        let inventory = p
            .frame(
                json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}),
                &mut stdin,
            )
            .unwrap();
        assert!(inventory.contains(&Event::Background(Vec::new())));
        assert!(inventory.contains(&Event::TurnActivity(TurnActivity::Completed)));
        // The grace period only keeps the wire open for a possible wake.
        p.claude_drain_at = Some(Instant::now() - Duration::from_secs(1));
        let subsequent = p.lifecycle_tick(Instant::now(), &mut stdin).unwrap();
        assert!(subsequent.contains(&Event::Done));
        assert!(p.done);
    }

    #[test]
    fn claude_shell_only_result_is_complete_and_still_receives_its_wake() {
        let mut p = protocol(Kind::Claude);
        let mut stdin = Vec::new();
        let inventory = p
            .frame(
                json!({"type":"system","subtype":"background_tasks_changed","tasks":[
                    {"task_id":"dev","task_type":"local_bash","command":"npm run dev"},
                    {"task_id":"watch","task_type":"monitor","ambient":true}
                ]}),
                &mut stdin,
            )
            .unwrap();
        assert!(inventory
            .iter()
            .any(|event| matches!(event, Event::Background(tasks)
            if tasks.len() == 1 && tasks[0].kind == TaskKind::Shell)));
        let result = p
            .frame(
                json!({"type":"result","result":"Started the server."}),
                &mut stdin,
            )
            .unwrap();
        assert!(result.contains(&Event::TurnActivity(TurnActivity::Completed)));
        assert!(!p.done, "the live shell still owns the transport");

        let ended = p.frame(json!({"type":"system","subtype":"task_notification","task_id":"dev","status":"completed"}), &mut stdin).unwrap();
        assert!(ended.contains(&Event::Background(Vec::new())));
        assert!(!ended
            .iter()
            .any(|event| matches!(event, Event::TurnActivity(_))));
        p.frame(
            json!({"type":"system","subtype":"init","session_id":"session"}),
            &mut stdin,
        )
        .unwrap();
        let replay = p
            .frame(
                json!({"type":"result","num_turns":0,"origin":{"kind":"task-notification"}}),
                &mut stdin,
            )
            .unwrap();
        assert!(!replay
            .iter()
            .any(|event| matches!(event, Event::Done | Event::TurnActivity(_))));
        let woke = p.frame(json!({"type":"assistant","message":{"content":[{"type":"text","text":"The server exited."}]}}), &mut stdin).unwrap();
        assert!(woke.contains(&Event::TurnActivity(TurnActivity::Running)));
        assert!(woke.contains(&Event::Text("The server exited.".into())));
        assert!(p
            .frame(
                json!({"type":"result","num_turns":1,"origin":{"kind":"task-notification"}}),
                &mut stdin
            )
            .unwrap()
            .contains(&Event::Done));
    }

    #[test]
    fn claude_waits_for_the_lead_to_wake_and_finishes_after_the_continuation() {
        let mut p = protocol(Kind::Claude);
        let mut stdin = Vec::new();
        p.frame(
            json!({"type":"system","subtype":"task_started","task_id":"review",
            "task_type":"local_agent","is_backgrounded":true}),
            &mut stdin,
        )
        .unwrap();
        let yielded = p
            .frame(json!({"type":"result","result":"Reviewing…"}), &mut stdin)
            .unwrap();
        assert!(
            yielded.contains(&Event::TurnActivity(TurnActivity::Yielded {
                wake_tasks: 1
            }))
        );

        let terminal = p.frame(json!({"type":"system","subtype":"task_updated","task_id":"review","patch":{"status":"completed"}}), &mut stdin).unwrap();
        assert!(terminal.contains(&Event::Background(Vec::new())));
        assert!(!terminal.contains(&Event::Done));
        let child = p
            .frame(
                json!({"type":"assistant","parent_tool_use_id":"review",
            "message":{"content":[{"type":"text","text":"Reviewed."}]}}),
                &mut stdin,
            )
            .unwrap();
        assert!(!child.contains(&Event::TurnActivity(TurnActivity::Running)));
        let lead = p.frame(json!({"type":"assistant","message":{"content":[{"type":"text","text":"Review passed."}]}}), &mut stdin).unwrap();
        assert!(lead.contains(&Event::TurnActivity(TurnActivity::Running)));
        assert!(lead.contains(&Event::Text("Review passed.".into())));
        assert!(p
            .frame(json!({"type":"result","num_turns":1}), &mut stdin)
            .unwrap()
            .contains(&Event::Done));
    }
}
