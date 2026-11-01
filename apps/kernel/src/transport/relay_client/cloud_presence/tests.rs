use std::sync::Mutex;

use tokio::sync::Notify;

use super::*;

#[derive(Default)]
struct FakeCloud {
    published: Mutex<Vec<bool>>,
    failures_left: Mutex<u32>,
    in_flight: Mutex<u32>,
    max_in_flight: Mutex<u32>,
    gate: Notify,
    gated: Mutex<bool>,
}

fn fake_publisher(cloud: Arc<FakeCloud>) -> CloudPresencePublisher {
    CloudPresencePublisher::start_with(move |online, _reason| {
        let cloud = Arc::clone(&cloud);
        async move {
            {
                let mut in_flight = cloud.in_flight.lock().unwrap();
                *in_flight += 1;
                let mut max = cloud.max_in_flight.lock().unwrap();
                *max = (*max).max(*in_flight);
            }
            if *cloud.gated.lock().unwrap() {
                cloud.gate.notified().await;
            }
            *cloud.in_flight.lock().unwrap() -= 1;
            let mut failures_left = cloud.failures_left.lock().unwrap();
            if *failures_left > 0 {
                *failures_left -= 1;
                return Err("cloud unreachable".to_string());
            }
            cloud.published.lock().unwrap().push(online);
            Ok(())
        }
    })
}

async fn settle() {
    for _ in 0..20 {
        tokio::task::yield_now().await;
    }
}

#[tokio::test(start_paused = true)]
async fn failed_presence_publish_retries_within_seconds_without_a_new_heartbeat() {
    let cloud = Arc::new(FakeCloud::default());
    *cloud.failures_left.lock().unwrap() = 2;
    let publisher = fake_publisher(Arc::clone(&cloud));

    publisher.publish(true, "relay registration sent");
    settle().await;
    assert!(cloud.published.lock().unwrap().is_empty());

    tokio::time::sleep(Duration::from_millis(3_100)).await;
    settle().await;
    assert_eq!(*cloud.published.lock().unwrap(), vec![true]);
}

#[tokio::test(start_paused = true)]
async fn newest_presence_intent_wins_and_publishes_never_overlap() {
    let cloud = Arc::new(FakeCloud::default());
    *cloud.gated.lock().unwrap() = true;
    let publisher = fake_publisher(Arc::clone(&cloud));

    publisher.publish(false, "relay connect timed out");
    settle().await;
    publisher.publish(false, "relay connect timed out");
    publisher.publish(true, "relay registration sent");
    settle().await;
    *cloud.gated.lock().unwrap() = false;
    cloud.gate.notify_one();
    settle().await;

    assert_eq!(*cloud.published.lock().unwrap(), vec![false, true]);
    assert_eq!(*cloud.max_in_flight.lock().unwrap(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_newer_intent_replaces_a_failing_one_instead_of_waiting_for_its_retry() {
    let cloud = Arc::new(FakeCloud::default());
    *cloud.failures_left.lock().unwrap() = 1;
    let publisher = fake_publisher(Arc::clone(&cloud));

    publisher.publish(false, "relay connect timed out");
    settle().await;
    publisher.publish(true, "relay registration sent");
    settle().await;

    assert_eq!(*cloud.published.lock().unwrap(), vec![true]);
}

#[test]
fn presence_retry_delay_is_prompt_and_bounded() {
    assert_eq!(presence_retry_delay(0), Duration::from_secs(1));
    assert_eq!(presence_retry_delay(1), Duration::from_secs(2));
    assert_eq!(presence_retry_delay(2), Duration::from_secs(4));
    assert_eq!(presence_retry_delay(9), Duration::from_secs(5));
}
