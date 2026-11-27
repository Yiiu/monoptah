use super::*;

fn stop_fake_codex(scenario: &str, mut observe: impl FnMut(&Event)) -> (Duration, Vec<Event>) {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("fake.py");
    std::fs::write(
        &fake,
        r#"#!/usr/bin/env python3
import json
import os
import sys
import time

def send(payload):
    print(json.dumps(payload), flush=True)

def finished(thread):
    send({"method":"turn/completed","params":{
        "threadId":thread,"turn":{"id":"turn","status":"interrupted"}
    }})

scenario = os.environ["MONOCODE_CODEX_STOP_TEST"]
for line in sys.stdin:
    request = json.loads(line)
    method = request.get("method")
    if method == "initialize":
        send({"id":request["id"],"result":{}})
    elif method == "thread/start":
        send({"id":request["id"],"result":{"thread":{"id":"thread"}}})
    elif method == "turn/start":
        send({"id":request["id"],"result":{"turn":{"id":"turn"}}})
    elif method == "turn/steer":
        steer = request["id"]
        if scenario == "steer-immediately":
            send({"id":steer,"result":{"turnId":"turn"}})
        else:
            send({"method":"item/agentMessage/delta","params":{"itemId":"armed","delta":"ready"}})
    elif method == "turn/interrupt":
        if scenario != "no-ack":
            send({"id":request["id"],"result":{}})
        if scenario == "late-steer":
            finished("another-thread")
            send({"id":steer,"result":{"turnId":"turn"}})
        elif scenario != "steer-immediately":
            time.sleep(9)
        finished("thread")
"#,
    )
    .unwrap();
    #[cfg(unix)]
    let binary = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
        fake
    };
    #[cfg(windows)]
    let binary = {
        let python =
            crate::scm::shell::which_in_path(&std::env::var("PATH").unwrap_or_default(), "python")
                .expect("the simulated Codex needs Python");
        let launcher = dir.path().join("fake.cmd");
        std::fs::write(
            &launcher,
            format!("@echo off\r\n\"{}\" \"%~dp0fake.py\"\r\n", python.display()),
        )
        .unwrap();
        launcher
    };
    let mut settings = Settings::default();
    settings
        .environment
        .insert("MONOCODE_CODEX_STOP_TEST".into(), scenario.into());
    let request = Request {
        kind: Kind::Codex,
        binary,
        cwd: dir.path().into(),
        prompt: "hello".into(),
        provider_id: None,
        model: None,
        settings,
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let (controls, receive) = mpsc::channel();
    let mut stop_at = None;
    let mut events = Vec::new();
    let result = run_with_controls(request, cancel.clone(), Some(receive), |batch| {
        for event in batch {
            match &event {
                Event::Ready if matches!(scenario, "late-steer" | "steer-immediately") => {
                    let message = crate::follow_up::Message {
                        id: "late".into(),
                        text: "keep this message".into(),
                        ..Default::default()
                    };
                    controls
                        .send(Control {
                            id: message.id.clone(),
                            decision: Decision::Steer(message),
                        })
                        .unwrap();
                }
                Event::Ready if stop_at.is_none() => {
                    stop_at = Some(Instant::now());
                    cancel.store(true, Ordering::Relaxed);
                }
                Event::Text(text) if text == "ready" && stop_at.is_none() => {
                    stop_at = Some(Instant::now());
                    cancel.store(true, Ordering::Relaxed);
                }
                Event::FollowUp(_) if scenario == "steer-immediately" => {
                    stop_at = Some(Instant::now());
                    cancel.store(true, Ordering::Relaxed);
                }
                _ => {}
            }
            observe(&event);
            events.push(event);
        }
        Ok(())
    });
    assert_eq!(result.unwrap_err(), "Stopped");
    (stop_at.unwrap().elapsed(), events)
}

#[test]
fn codex_stop_keeps_a_steer_ack_after_the_interrupt_ack() {
    let (_, events) = stop_fake_codex("late-steer", |_| {});
    assert!(events
        .iter()
        .any(|event| matches!(event, Event::FollowUp(message) if message.id == "late")));
}

#[test]
fn codex_stop_with_no_interrupt_reply_has_a_deadline() {
    let (elapsed, events) = stop_fake_codex("no-ack", |_| {});
    assert!(elapsed < Duration::from_secs(8), "Stop took {elapsed:?}");
    assert!(events.contains(&Event::StopTimedOut {
        stage: "acknowledgement"
    }));
}

#[test]
fn codex_stop_with_only_an_ack_waits_for_a_terminal_event_with_a_deadline() {
    let (elapsed, events) = stop_fake_codex("ack-only", |_| {});
    assert!(
        (Duration::from_secs(2)..Duration::from_secs(8)).contains(&elapsed),
        "Stop took {elapsed:?}"
    );
    assert!(events.contains(&Event::StopTimedOut { stage: "terminal" }));
}

#[test]
fn a_codex_stop_wait_does_not_delay_steer_delivery_in_another_turn() {
    let (second_ready, ready) = mpsc::channel();
    let (allow_steer, start) = mpsc::channel();
    let first_timed_out = Arc::new(AtomicBool::new(false));
    let timeout_at_receipt = first_timed_out.clone();
    let second = thread::spawn(move || {
        let mut timed_out = None;
        let (_, events) = stop_fake_codex("steer-immediately", |event| match event {
            Event::Ready => {
                second_ready.send(()).unwrap();
                start.recv_timeout(Duration::from_secs(15)).unwrap();
            }
            Event::FollowUp(_) => timed_out = Some(timeout_at_receipt.load(Ordering::SeqCst)),
            _ => {}
        });
        (timed_out, events)
    });
    ready.recv_timeout(Duration::from_secs(15)).unwrap();
    let first = thread::spawn(move || {
        stop_fake_codex("ack-only", |event| match event {
            Event::Ready => allow_steer.send(()).unwrap(),
            Event::StopTimedOut { .. } => first_timed_out.store(true, Ordering::SeqCst),
            _ => {}
        })
    });
    let (timed_out_at_receipt, second_events) = second.join().unwrap();
    let (_, first_events) = first.join().unwrap();
    assert_eq!(timed_out_at_receipt, Some(false));
    assert!(first_events.contains(&Event::StopTimedOut { stage: "terminal" }));
    assert!(second_events
        .iter()
        .any(|event| matches!(event, Event::FollowUp(message) if message.id == "late")));
}
