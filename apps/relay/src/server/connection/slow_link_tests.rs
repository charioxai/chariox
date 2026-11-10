//! MP-08 / MP-09 / MP-10 / MP-11: the relay writer in front of a throttled link
//! whose queue never refuses a frame (the bufferbloat case), in paused time.
//! Small traffic must wait about a round trip behind any amount of bulk, and
//! the window must still let one transfer use the whole link.
use super::*;
use crate::outgoing::RelaySender;
use std::time::Duration;
use tokio::time::Instant;

const MB: usize = 1_000_000;

struct Link {
    sender: RelaySender,
    /// Every reassembled message and when the peer had it.
    delivered: mpsc::UnboundedReceiver<(Instant, String)>,
}

/// Frames the writer writes wait in a queue drained at `rate` bytes/s; the peer
/// reassembles each frame half a round trip after it leaves the queue, and its
/// receipt reaches the writer half a round trip later.
fn link(rate: f64, rtt: Duration) -> Link {
    let (wire_tx, mut wire) = mpsc::unbounded_channel::<(Instant, Message)>();
    let sink = Box::pin(futures_util::sink::unfold(
        wire_tx,
        |tx, message: Message| async move {
            let _ = tx.send((Instant::now(), message));
            Ok::<_, std::convert::Infallible>(tx)
        },
    ));
    let (_control, controls) = mpsc::channel(8);
    let (sender, outgoing) = RelaySender::channel(1024);
    let (ack_tx, ack_rx) = mpsc::channel(crate::frame_transport::MAX_UNACKED_FRAMES);
    let queue = sender.queue();
    tokio::spawn(run_writer(
        sink,
        outgoing,
        true,
        false,
        false,
        ack_rx,
        controls,
        WriterUsage::fixture_with_queue(true, queue),
    ));
    let (arrive_tx, mut arrive) = mpsc::unbounded_channel::<(Instant, Message)>();
    tokio::spawn(async move {
        // Absolute deadlines: per-frame sleeps would round each frame up to the timer tick.
        let mut free_at = Instant::now();
        while let Some((queued_at, message)) = wire.recv().await {
            free_at = free_at.max(queued_at) + Duration::from_secs_f64(message.len() as f64 / rate);
            tokio::time::sleep_until(free_at).await;
            let _ = arrive_tx.send((free_at + rtt / 2, message));
        }
    });
    let (receipt_tx, mut receipts) = mpsc::unbounded_channel::<(Instant, (u32, u32))>();
    let (delivered_tx, delivered) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut receiver = crate::frame_transport::FrameReceiver::default();
        while let Some((at, message)) = arrive.recv().await {
            tokio::time::sleep_until(at).await;
            let Message::Text(text) = message else {
                panic!("text frames only")
            };
            let (complete, receipt) = receiver.receive(&text).expect("a valid frame");
            if let Some(receipt) = receipt {
                let TransportFrame::TransportAck {
                    transfer_id,
                    offset,
                } = serde_json::from_str(&receipt).unwrap()
                else {
                    panic!("receipt expected")
                };
                let _ = receipt_tx.send((at + rtt / 2, (transfer_id, offset)));
            }
            if let Some(complete) = complete {
                let _ = delivered_tx.send((at, complete));
            }
        }
    });
    tokio::spawn(async move {
        while let Some((at, receipt)) = receipts.recv().await {
            tokio::time::sleep_until(at).await;
            if ack_tx.send(receipt).await.is_err() {
                return;
            }
        }
    });
    Link { sender, delivered }
}

fn bulk(kind: &str, id: &str, bytes: usize) -> Message {
    let field = if kind == "client_event" {
        "subscription_id"
    } else {
        "request_id"
    };
    Message::Text(
        serde_json::json!({"kind": kind, field: id, "payload": "y".repeat(bytes)})
            .to_string()
            .into(),
    )
}

/// Queue `messages` in the background, retrying while the receiver's bulk budget is full.
fn feed(sender: &RelaySender, messages: Vec<Message>) {
    let sender = sender.clone();
    tokio::spawn(async move {
        for message in messages {
            while sender.try_send(message.clone()).is_err() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
    });
}

struct Outcome {
    /// Completion time of each bulk message, by id.
    finished: Vec<(String, Duration)>,
    /// Longest wait of a small reply from queueing to the peer.
    worst_small: Duration,
    small_count: usize,
}

/// Runs `bulk` through the link, queueing a small reply every 2 s until every
/// bulk message reached the peer intact.
async fn run(rate: f64, rtt: Duration, messages: Vec<(String, Message)>) -> Outcome {
    let start = Instant::now();
    let mut link = link(rate, rtt);
    let expected: Vec<(String, usize)> = messages
        .iter()
        .map(|(id, message)| (id.clone(), message.len()))
        .collect();
    feed(&link.sender, messages.into_iter().map(|(_, m)| m).collect());
    let mut small_sent = std::collections::HashMap::new();
    let mut probe = tokio::time::interval(Duration::from_secs(2));
    let mut finished = Vec::new();
    let mut worst_small = Duration::ZERO;
    let mut small_count = 0;
    while finished.len() < expected.len() {
        tokio::select! {
            _ = probe.tick() => {
                let id = format!("small-{}", small_sent.len());
                let small = serde_json::json!({"kind": "client_response", "request_id": id, "payload": "ok"});
                link.sender.try_send(Message::Text(small.to_string().into())).expect("small replies are always admitted");
                small_sent.insert(id, Instant::now());
            }
            delivered = link.delivered.recv() => {
                let (at, text) = delivered.expect("the writer ended");
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                let id = value.get("request_id").or_else(|| value.get("subscription_id")).and_then(|v| v.as_str()).unwrap().to_string();
                if let Some(sent) = small_sent.get(&id) {
                    worst_small = worst_small.max(at.duration_since(*sent));
                    small_count += 1;
                    continue;
                }
                let (_, length) = expected.iter().find(|(e, _)| *e == id).expect("a known message");
                assert_eq!(text.len(), *length, "{id} arrived intact");
                finished.push((id, at.duration_since(start)));
            }
        }
    }
    Outcome {
        finished,
        worst_small,
        small_count,
    }
}

// A small reply on a 64 KB/s, 40 ms link carrying 20 MB plus three more bulk
// transfers reaches the peer within two round trips plus two chunk times
// (0.6 s), for the whole transfer; nothing is lost.
#[tokio::test(start_paused = true)]
async fn a_small_reply_waits_about_a_round_trip_behind_any_bulk_on_a_slow_link() {
    const RATE: f64 = 64_000.0;
    const RTT: Duration = Duration::from_millis(40);
    let messages = vec![
        (
            "bulk-reply".into(),
            bulk("client_response", "bulk-reply", 20 * MB),
        ),
        ("s1".into(), bulk("client_event", "s1", 2 * MB)),
        ("s2".into(), bulk("client_event", "s2", 2 * MB)),
        (
            "bulk-request".into(),
            bulk("daemon_request", "bulk-request", 2 * MB),
        ),
    ];
    let outcome = run(RATE, RTT, messages).await;
    let bound = 2 * RTT
        + Duration::from_secs_f64(2.0 * CHUNK_BYTES as f64 / RATE)
        + RTT / 2
        + Duration::from_millis(20);
    println!(
        "slow link: {} small replies, worst {:?} (bound {bound:?}); finished {:?}",
        outcome.small_count, outcome.worst_small, outcome.finished
    );
    assert!(outcome.small_count > 100);
    assert!(
        outcome.worst_small <= bound,
        "a small reply waited {:?} behind bulk, bound {bound:?}",
        outcome.worst_small
    );
    let total: usize = 26 * MB;
    let elapsed = outcome.finished.iter().map(|(_, t)| *t).max().unwrap();
    let rate = total as f64 / elapsed.as_secs_f64();
    assert!(rate >= 0.9 * RATE, "the link carried {rate:.0} B/s");
}

// Gateway file chunks are replies: three bulk replies at once fill both reply
// slots, and a small reply must still pass them (relayproof 1m three: 6.9 s p95).
#[tokio::test(start_paused = true)]
async fn a_small_reply_passes_three_bulk_replies() {
    const RATE: f64 = 64_000.0;
    const RTT: Duration = Duration::from_millis(40);
    let messages = (0..3)
        .map(|n| {
            let id = format!("bulk-{n}");
            let message = bulk("client_response", &id, MB);
            (id, message)
        })
        .collect();
    let outcome = run(RATE, RTT, messages).await;
    let bound = 2 * RTT
        + Duration::from_secs_f64(2.0 * CHUNK_BYTES as f64 / RATE)
        + RTT / 2
        + Duration::from_millis(20);
    println!(
        "three bulk replies: {} small replies, worst {:?} (bound {bound:?})",
        outcome.small_count, outcome.worst_small
    );
    assert!(
        outcome.worst_small <= bound,
        "a small reply waited {:?} behind bulk replies, bound {bound:?}",
        outcome.worst_small
    );
}

// The window must not strangle a long transfer: one 20 MB reply alone uses at
// least 90 % of a 64 KB/s link.
#[tokio::test(start_paused = true)]
async fn one_transfer_uses_the_whole_slow_link() {
    const RATE: f64 = 64_000.0;
    let outcome = run(
        RATE,
        Duration::from_millis(40),
        vec![(
            "bulk-reply".into(),
            bulk("client_response", "bulk-reply", 20 * MB),
        )],
    )
    .await;
    let rate = 20.0 * MB as f64 / outcome.finished[0].1.as_secs_f64();
    println!(
        "slow link alone: {rate:.0} B/s of {RATE}, worst small {:?}",
        outcome.worst_small
    );
    assert!(rate >= 0.9 * RATE, "one transfer got {rate:.0} B/s");
}

// A fast long link (50 Mbit/s, 90 ms: 560 KB in flight) is filled: one 50 MB
// reply gets at least 90 % of it, window growth included.
#[tokio::test(start_paused = true)]
async fn one_transfer_fills_a_fast_long_link() {
    const RATE: f64 = 6_250_000.0;
    let outcome = run(
        RATE,
        Duration::from_millis(90),
        vec![(
            "bulk-reply".into(),
            bulk("client_response", "bulk-reply", 50 * MB),
        )],
    )
    .await;
    let rate = 50.0 * MB as f64 / outcome.finished[0].1.as_secs_f64();
    println!(
        "fast long link: {rate:.0} B/s of {RATE}, worst small {:?}",
        outcome.worst_small
    );
    assert!(rate >= 0.9 * RATE, "one transfer got {rate:.0} B/s");
}

// In-process, unthrottled, real time: the writer's own throughput with
// immediate receipts. Printed for the head-to-head benchmark; the assertion
// only catches a writer that stalls.
#[tokio::test]
async fn unthrottled_in_process_throughput() {
    let (wire_tx, mut wire) = mpsc::unbounded_channel::<Message>();
    let sink = Box::pin(futures_util::sink::unfold(
        wire_tx,
        |tx, message: Message| async move {
            let _ = tx.send(message);
            Ok::<_, std::convert::Infallible>(tx)
        },
    ));
    let (_control, controls) = mpsc::channel(8);
    let (sender, outgoing) = RelaySender::channel(1024);
    let (ack_tx, ack_rx) = mpsc::channel(crate::frame_transport::MAX_UNACKED_FRAMES);
    let queue = sender.queue();
    tokio::spawn(run_writer(
        sink,
        outgoing,
        true,
        false,
        false,
        ack_rx,
        controls,
        WriterUsage::fixture_with_queue(true, queue),
    ));
    let total = 64 * MB;
    let message = bulk("client_response", "bulk", total);
    let started = std::time::Instant::now();
    feed(&sender, vec![message]);
    let mut receiver = crate::frame_transport::FrameReceiver::default();
    loop {
        let Some(Message::Text(text)) = wire.recv().await else {
            panic!("text frames only")
        };
        let (complete, receipt) = receiver.receive(&text).unwrap();
        if let Some(receipt) = receipt {
            let TransportFrame::TransportAck {
                transfer_id,
                offset,
            } = serde_json::from_str(&receipt).unwrap()
            else {
                panic!("receipt expected")
            };
            ack_tx.send((transfer_id, offset)).await.unwrap();
        }
        if complete.is_some() {
            break;
        }
    }
    let rate = total as f64 / started.elapsed().as_secs_f64() / MB as f64;
    println!("unthrottled in-process: {rate:.1} MB/s");
    assert!(rate > 10.0, "{rate:.1} MB/s");
}
