//! Lifecycle rules from the desktop adapters. An idle stream is not a failed turn.
use super::*;
static CONTROL_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
const CODEX_INTERRUPT_ACK_TIMEOUT: Duration = Duration::from_secs(3);
const CODEX_INTERRUPT_TERMINAL_TIMEOUT: Duration = Duration::from_secs(5);

impl Protocol {
    /// An RPC reply confirms only that the interrupt request was handled.
    /// Keep the old writer until its exact turn ends or it is retired.
    pub(super) fn settle_codex_stop(
        &mut self,
        wire: &mut parked::Wire,
        pending: &mut Vec<Event>,
    ) -> bool {
        let trace = |phase: &str, turn: Option<&str>| {
            if std::env::var_os("MONOCODE_NATIVE_TRACE").is_some() {
                eprintln!("provider-stop phase={phase} turn_id={}", turn.unwrap_or(""));
            }
        };
        let mut deadline = Instant::now() + CODEX_INTERRUPT_ACK_TIMEOUT;
        wire.stdin.write_deadline = Some(deadline);
        let interrupted = self.interrupt(&mut wire.stdin);
        wire.stdin.write_deadline = None;
        if interrupted != Ok(true) {
            if interrupted.is_err() && Instant::now() >= deadline {
                pending.push(Event::StopTimedOut {
                    stage: "acknowledgement",
                });
            }
            trace("interrupt_not_written", self.active_turn.as_deref());
            return false;
        }
        trace("interrupt_written", self.active_turn.as_deref());

        let mut stage = "acknowledgement";
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let result = if remaining.is_zero() {
                Err(mpsc::RecvTimeoutError::Timeout)
            } else {
                wire.rx.recv_timeout(remaining)
            };
            let bytes = match result {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(_)) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                    trace("disconnected", self.active_turn.as_deref());
                    return false;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    trace(stage, self.active_turn.as_deref());
                    pending.push(Event::StopTimedOut { stage });
                    return false;
                }
            };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if value["id"].as_str() == self.interrupt_id.as_deref() && stage == "acknowledgement" {
                let rejected = value.get("error").is_some_and(|error| !error.is_null());
                if rejected || value.get("result").is_some() {
                    trace(
                        if rejected {
                            "interrupt_rejected"
                        } else {
                            "interrupt_acknowledged"
                        },
                        self.active_turn.as_deref(),
                    );
                    stage = "terminal";
                    deadline = Instant::now() + CODEX_INTERRUPT_TERMINAL_TIMEOUT;
                }
                continue;
            }
            if let Some(events) = self.steer_response(&value) {
                pending.extend(events);
            }
            let params = &value["params"];
            match value["method"].as_str() {
                Some("item/started" | "item/completed") => {
                    pending.extend(self.codex_steer_echo(params, &params["item"]));
                }
                Some("turn/completed" | "turn/aborted") => {
                    let turn = params["turnId"]
                        .as_str()
                        .or_else(|| params["turn"]["id"].as_str());
                    if turn.is_some()
                        && turn == self.active_turn.as_deref()
                        && params["threadId"].as_str().is_none_or(|thread| {
                            Some(thread) == self.request.provider_id.as_deref()
                        })
                    {
                        pending.extend(self.codex_terminal_steers(&value));
                        self.done = true;
                        trace("terminal_received", turn);
                        return true;
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn codex_elicitation(
        &mut self,
        wire: Value,
        mut request: Value,
        events: &mut Vec<Event>,
        stdin: &mut impl Write,
    ) -> Result<(), String> {
        let confirmation = mcp_confirmation(&request);
        let Some((title, content)) = confirmation else {
            events.push(Event::Status("This MCP server requested a form or browser sign-in that Monoptah does not support yet. Complete it in the server's own interface.".into()));
            return write_json(
                stdin,
                json!({"id":wire,"result":{"action":"cancel","content":null,"_meta":null}}),
            );
        };
        if !self.request.settings.plan && self.request.settings.access == AccessMode::FullAccess {
            return write_json(
                stdin,
                json!({"id":wire,"result":{"action":"accept","content":content,"_meta":null}}),
            );
        }
        request["nativeContent"] = content;
        let id = wire.to_string();
        self.approvals
            .insert(id.clone(), (wire, "codex-mcp".into(), request));
        events.push(Event::Approval(Approval {
            id,
            title,
            detail: String::new(),
            can_remember: false,
            can_allow_once: true,
        }));
        Ok(())
    }

    pub(super) fn claude_prompt(&mut self, stdin: &mut impl Write) -> Result<(), String> {
        if self.claude_prompt_sent {
            return Ok(());
        }
        let prompt = if self.request.settings.effort.as_deref() == Some("ultrathink") {
            if self.request.prompt.is_empty() {
                "Ultrathink:".into()
            } else {
                format!("Ultrathink:\n{}", self.request.prompt)
            }
        } else {
            self.request.prompt.clone()
        };
        if !self.probe && !self.command_probe && !self.request.settings.text_only {
            self.claude_goal = goal::TranscriptCursor::before_prompt(&self.request);
        }
        write_json(
            stdin,
            json!({"type":"user","session_id":"","parent_tool_use_id":null,
                "message":{"role":"user","content":self.request.claude_content(&prompt)?}}),
        )?;
        self.claude_prompt_sent = true;
        self.follow_up_ready = true;
        Ok(())
    }

    pub(super) fn lifecycle_tick(
        &mut self,
        started: Instant,
        stdin: &mut impl Write,
    ) -> Result<Vec<Event>, String> {
        let mut events = self.expire_questions(stdin)?;
        if self.request.kind.is_acp() {
            events.extend(self.acp_tick(stdin)?);
        } else if self.request.kind.is_pi() {
            events.extend(self.pi_tick(stdin)?);
        } else if self.request.kind == Kind::Claude {
            // The desktop adapter proceeds after eight seconds even when older
            // CLIs never acknowledge the initialize control request.
            if !self.claude_prompt_sent && started.elapsed() >= Duration::from_secs(8) {
                self.claude_prompt(stdin)?;
            }
            self.sync_claude_result(&mut events);
        }
        Ok(events)
    }

    pub(super) fn set_claude_turn_activity(&mut self, next: TurnActivity, events: &mut Vec<Event>) {
        if self.claude_turn_activity != next {
            self.claude_turn_activity = next;
            events.push(Event::TurnActivity(next));
        }
    }

    pub(super) fn sync_claude_result(&mut self, events: &mut Vec<Event>) {
        self.sync_background(events);
        if !self.claude_result_seen || self.done {
            return;
        }
        let now = Instant::now();
        let wake_tasks = self
            .activities
            .claude_tasks
            .iter()
            .filter(|task| task.kind != TaskKind::Shell)
            .count() as u32;
        let wake_tasks = wake_tasks.max(u32::from(
            self.claude_task_wake_at.is_some_and(|at| at > now),
        ));
        if wake_tasks > 0 {
            self.set_claude_turn_activity(TurnActivity::Yielded { wake_tasks }, events);
        } else if self.claude_resume_at.is_some_and(|at| at > now) {
            self.set_claude_turn_activity(TurnActivity::Running, events);
        } else if !self.activities.claude_tasks.is_empty()
            || self.claude_drain_at.is_some_and(|at| at > now)
        {
            // Completing the visible turn does not close the wire. The same
            // CLI can still wake when a shell emits its background result.
            self.set_claude_turn_activity(TurnActivity::Completed, events);
        } else {
            self.done = true;
            events.push(Event::Done);
        }
    }

    /// A user Stop uses the provider's cancellation API before transport teardown.
    /// Returns whether there is an active request whose acknowledgement can settle.
    pub(super) fn interrupt(&mut self, stdin: &mut impl Write) -> Result<bool, String> {
        let interrupt_id = format!(
            "native-interrupt-{}",
            CONTROL_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        self.interrupt_id = Some(interrupt_id.clone());
        match self.request.kind {
            Kind::Codex => {
                self.approvals.clear();
                self.questions.clear();
                if let (Some(thread), Some(turn)) = (&self.request.provider_id, &self.active_turn) {
                    write_json(
                        stdin,
                        json!({"id":interrupt_id,"method":"turn/interrupt","params":{"threadId":thread,"turnId":turn}}),
                    )?;
                    return Ok(true);
                }
                Ok(false)
            }
            Kind::Claude => {
                // Deny outstanding permissions and skip questions before interrupt,
                // exactly as cancelClaudeTurn does. Stop includes background tasks.
                for id in self.approvals.keys().cloned().collect::<Vec<_>>() {
                    self.respond(
                        Control {
                            id,
                            decision: Decision::Deny,
                        },
                        stdin,
                    )?;
                }
                for id in self.questions.keys().cloned().collect::<Vec<_>>() {
                    self.respond(
                        Control {
                            id,
                            decision: Decision::Deny,
                        },
                        stdin,
                    )?;
                }
                let tasks: Vec<_> = self.activities.claude_background.iter().cloned().collect();
                for (ix, task) in tasks.iter().enumerate() {
                    write_json(
                        stdin,
                        json!({"type":"control_request","request_id":format!("native-stop-task-{ix}"),"request":{"subtype":"stop_task","task_id":task}}),
                    )?;
                }
                write_json(
                    stdin,
                    json!({"type":"control_request","request_id":interrupt_id,"request":{"subtype":"interrupt"}}),
                )?;
                Ok(self.claude_prompt_sent)
            }
            kind if kind.is_pi() => {
                self.pi_cancel(stdin)?;
                Ok(self.follow_up_ready)
            }
            _ => {
                let active = self.acp_active_session().is_some();
                self.acp_cancel(stdin)?;
                Ok(active)
            }
        }
    }

    /// `None` when the session uses Codex's own settings: no approval or sandbox parameters.
    pub(super) fn codex_access(&self) -> Option<(Value, &'static str, Value, &'static str)> {
        let (policy, sandbox, mut sandbox_policy, reviewer) =
            self.request.settings.access.codex()?;
        if (self.request.settings.operator
            || self.request.settings.operator_enabled
            || self.request.settings.orchestrator)
            && sandbox_policy["type"] != "dangerFullAccess"
        {
            sandbox_policy["networkAccess"] = true.into();
        }
        Some((policy, sandbox, sandbox_policy, reviewer))
    }
}

fn mcp_confirmation(request: &Value) -> Option<(String, Value)> {
    if !matches!(
        request["mode"].as_str(),
        Some("form" | "openai/form" | "openaiForm")
    ) {
        return None;
    }
    let schema = request["requestedSchema"].as_object()?;
    if schema.get("type")?.as_str()? != "object"
        || schema.keys().any(|key| {
            ![
                "type",
                "properties",
                "required",
                "title",
                "description",
                "$schema",
                "additionalProperties",
            ]
            .contains(&key.as_str())
        })
    {
        return None;
    }
    let properties = schema.get("properties")?.as_object()?;
    if properties.len() > 1 {
        return None;
    }
    if let Some(required) = schema.get("required") {
        if required
            .as_array()?
            .iter()
            .any(|key| key.as_str().is_none_or(|key| !properties.contains_key(key)))
        {
            return None;
        }
    }
    let mut content = serde_json::Map::new();
    let mut detail = String::new();
    if let Some((key, field)) = properties.iter().next() {
        let field = field.as_object()?;
        if field.get("type")?.as_str()? != "boolean"
            || field
                .keys()
                .any(|key| !["type", "title", "description", "default"].contains(&key.as_str()))
        {
            return None;
        }
        detail = field
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_owned();
        if let Some(description) = field
            .get("description")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            detail.push_str(" — ");
            detail.push_str(description);
        }
        content.insert(key.clone(), true.into());
    }
    let mut title = format!(
        "{}: {}",
        request["serverName"].as_str().unwrap_or("MCP"),
        request["message"].as_str().unwrap_or("Approve request")
    );
    if !detail.is_empty() {
        title.push_str(" — ");
        title.push_str(&detail);
    }
    Some((title, content.into()))
}

pub(super) fn recoverable_codex_resume(error: &str) -> bool {
    let message = error.to_lowercase();
    message.contains("thread")
        && [
            "not found",
            "unknown thread",
            "no such thread",
            "does not exist",
            "missing thread",
            "thread id",
        ]
        .iter()
        .any(|part| message.contains(part))
}

pub(super) fn claude_session_id() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = format!("{:032x}", u128::from_be_bytes(bytes));
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}
