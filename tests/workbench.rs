use serde_json::json;
use serial_mcp_server::workbench::{
    Event, EventQuery, EventRing, Hub, OpenRequest, SendRequest, TaskInfo,
};
use std::{sync::Arc, time::Duration};

fn demo() -> OpenRequest {
    serde_json::from_value(json!({"port":"DEMO","baud_rate":115200,"demo":true})).unwrap()
}
fn send(data: &str, wait: Option<&str>) -> SendRequest {
    serde_json::from_value(json!({"data":data,"wait_for":wait,"timeout_ms":2000,"source":"test"}))
        .unwrap()
}
async fn finished(hub: &Hub, id: &str) -> TaskInfo {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let info = hub.task(id).await.unwrap();
            if info.finished_at.is_some() {
                return info;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
async fn running(hub: &Hub, id: &str) {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if hub.task(id).await.unwrap().status == "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn shared_readers_and_command_expectation_do_not_consume_capture() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new(dir.path().into()).await.unwrap();
    let session = hub.open(demo()).await.unwrap();
    assert!(hub.open(demo()).await.is_err());
    let task = hub
        .send(&session.id, send("hello\r\n", Some("OK")))
        .await
        .unwrap();
    let task = finished(&hub, &task.id).await;
    assert_eq!(task.status, "succeeded");
    let q = EventQuery {
        after: 0,
        limit: Some(1000),
        contains: None,
    };
    let first = hub.events(&session.id, &q, false).await.unwrap();
    let second = hub.events(&session.id, &q, false).await.unwrap();
    assert_eq!(first.events[0].seq, second.events[0].seq);
    for event in &first.events {
        assert!(second
            .events
            .iter()
            .any(|other| other.seq == event.seq && other.base64 == event.base64));
    }
    assert!(first
        .events
        .iter()
        .any(|e| e.direction == "rx" && e.text.contains("hello")));
    assert!(!first.gap);
    hub.close(&session.id).await.unwrap();
    let history = hub.events(&session.id, &q, true).await.unwrap();
    assert!(history
        .events
        .iter()
        .any(|e| e.direction == "tx" && e.text == "hello\r\n"));
    let raw = std::fs::read(dir.path().join(&session.id).join("rx.log")).unwrap();
    assert!(String::from_utf8_lossy(&raw).contains("hello"));
    let restarted = Hub::new(dir.path().into()).await.unwrap();
    assert_eq!(restarted.list().await.unwrap()[0].state, "closed");
    assert!(
        restarted
            .events(&session.id, &q, false)
            .await
            .unwrap()
            .events
            .len()
            >= first.events.len()
    );
}

#[tokio::test]
async fn cancelled_queued_command_is_never_transmitted_and_queue_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new(dir.path().into()).await.unwrap();
    let s = hub.open(demo()).await.unwrap();
    let first = hub
        .send(&s.id, send("first", Some("THIS_WILL_NOT_MATCH")))
        .await
        .unwrap();
    running(&hub, &first.id).await;
    let second = hub
        .send(&s.id, send("MUST_NOT_BE_SENT", None))
        .await
        .unwrap();
    hub.cancel_task(&second.id).await.unwrap();
    hub.cancel_task(&first.id).await.unwrap();
    assert_eq!(finished(&hub, &first.id).await.status, "cancelled");
    assert_eq!(finished(&hub, &second.id).await.status, "cancelled");
    let third = hub.send(&s.id, send("third", Some("OK"))).await.unwrap();
    assert_eq!(finished(&hub, &third.id).await.status, "succeeded");
    let page = hub
        .events(
            &s.id,
            &EventQuery {
                limit: Some(1000),
                ..Default::default()
            },
            false,
        )
        .await
        .unwrap();
    assert!(!page
        .events
        .iter()
        .any(|e| e.direction == "tx" && e.text.contains("MUST_NOT_BE_SENT")));
    hub.shutdown().await;
}

#[tokio::test]
async fn macro_holds_send_order_while_receive_and_other_readers_continue() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new(dir.path().into()).await.unwrap();
    let s = hub.open(demo()).await.unwrap();
    let request=serde_json::from_value(json!({"name":"sequence","source":"test","pack":{"schema_version":"0.3","name":"test","macros":[{"name":"sequence","steps":[{"type":"send","data":"alpha"},{"type":"expect","op":"contains","data":"OK"},{"type":"delay","ms":600},{"type":"send","data":"beta"},{"type":"expect","op":"contains","data":"OK"}]}]}})).unwrap();
    let a = hub.run_macro(&s.id, request).await.unwrap();
    running(&hub, &a.id).await;
    let b = hub.send(&s.id, send("gamma", None)).await.unwrap();
    assert_eq!(finished(&hub, &a.id).await.status, "succeeded");
    assert_eq!(finished(&hub, &b.id).await.status, "sent");
    let page = hub
        .events(
            &s.id,
            &EventQuery {
                limit: Some(1000),
                ..Default::default()
            },
            false,
        )
        .await
        .unwrap();
    let sent: Vec<_> = page
        .events
        .iter()
        .filter(|e| e.direction == "tx")
        .map(|e| e.text.as_str())
        .collect();
    assert_eq!(sent, vec!["alpha", "beta", "gamma"]);
    assert!(page
        .events
        .iter()
        .any(|e| e.direction == "rx" && e.text.contains("Detect(")));
    hub.shutdown().await;
}

#[tokio::test]
async fn close_cancels_tasks_and_flushes_final_state() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new(dir.path().into()).await.unwrap();
    let s = hub.open(demo()).await.unwrap();
    let task = hub
        .send(&s.id, send("waiting", Some("NEVER")))
        .await
        .unwrap();
    running(&hub, &task.id).await;
    let info = hub.close(&s.id).await.unwrap();
    assert_eq!(info.state, "closed");
    assert!(info.active_task.is_none());
    assert_eq!(hub.task(&task.id).await.unwrap().status, "cancelled");
    assert!(hub.send(&s.id, send("after-close", None)).await.is_err());
    let disk: serde_json::Value = serde_json::from_slice(
        &std::fs::read(dir.path().join(&s.id).join("session.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(disk["state"], "closed");
}

#[test]
fn cursor_pages_report_eviction_and_advance_past_filtered_events() {
    let mut ring = EventRing::default();
    for i in 0..4100 {
        ring.push(Event {
            seq: 0,
            time: "now".into(),
            direction: "rx".into(),
            source: "device".into(),
            text: format!("line {i}"),
            base64: String::new(),
            bytes: 0,
            rx_offset: 0,
            task_id: None,
        });
    }
    let a = ring.page(&EventQuery {
        after: 0,
        limit: Some(3),
        contains: None,
    });
    assert!(a.gap);
    assert_eq!(a.first_available, 5);
    assert_eq!(a.next_cursor, 7);
    assert!(a.has_more);
    let b = ring.page(&EventQuery {
        after: a.next_cursor,
        limit: Some(3),
        contains: None,
    });
    assert!(!b.gap);
    assert_eq!(b.events[0].seq, 8);
    let missing = ring.page(&EventQuery {
        after: 7,
        limit: Some(3),
        contains: Some("absent".into()),
    });
    assert!(missing.events.is_empty());
    assert_eq!(missing.next_cursor, 4100);
    assert!(!missing.has_more);
}

#[tokio::test]
async fn http_clients_share_one_session_and_reject_foreign_browser_writes() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new(dir.path().into()).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = serial_mcp_server::workbench::http::router(Arc::clone(&hub), address);
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    assert_eq!(
        client
            .post(format!("{base}/api/sessions"))
            .json(&demo())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(format!("{base}/api/sessions"))
            .header("x-serial-workbench", "1")
            .header("origin", "https://foreign.example")
            .json(&demo())
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let a = serial_mcp_server::workbench::client::WorkbenchClient::new(&base).unwrap();
    let b = a.clone();
    let opened = a
        .post("/api/sessions", serde_json::to_value(demo()).unwrap())
        .await
        .unwrap();
    let id = opened["id"].as_str().unwrap();
    assert_eq!(b.get("/api/sessions").await.unwrap()[0]["id"], id);
    let task = b
        .post(
            &format!("/api/sessions/{id}/send"),
            serde_json::to_value(send("http-shared", Some("OK"))).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        a.wait_task(task["id"].as_str().unwrap()).await.unwrap()["status"],
        "succeeded"
    );
    hub.shutdown().await;
    server.abort();
}

#[tokio::test]
async fn human_control_blocks_other_writers_but_keeps_readers_active() {
    let dir = tempfile::tempdir().unwrap();
    let hub = Hub::new(dir.path().into()).await.unwrap();
    let s = hub.open(demo()).await.unwrap();
    hub.takeover(&s.id).await.unwrap();
    assert!(hub
        .send(&s.id, send("blocked-ai", None))
        .await
        .unwrap_err()
        .to_string()
        .contains("control held"));
    let mut human = send("human-input", Some("OK"));
    human.source = "human".into();
    let task = hub.send(&s.id, human).await.unwrap();
    assert_eq!(finished(&hub, &task.id).await.status, "succeeded");
    assert!(!hub
        .events(&s.id, &EventQuery::default(), false)
        .await
        .unwrap()
        .events
        .is_empty());
    hub.release_control(&s.id).await.unwrap();
    let task = hub
        .send(&s.id, send("shared-again", Some("OK")))
        .await
        .unwrap();
    assert_eq!(finished(&hub, &task.id).await.status, "succeeded");
    hub.shutdown().await;
}
