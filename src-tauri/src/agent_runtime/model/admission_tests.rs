use super::{completion, gateway, raw, MockProvider};
use crate::agent_runtime::model::{
    admission::{admission_domain, gate_for, AdmissionGate, CLOUD_MAX_IN_FLIGHT},
    CancelToken, ModelError, ModelRequest, OpenAiCompatibleGateway,
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};

fn wait_for_queue(gate: &AdmissionGate, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while gate.queued() < count && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        gate.queued(),
        count,
        "request did not enter admission queue"
    );
}

#[test]
fn local_admission_is_fifo_and_cancelled_waiters_do_not_acquire() {
    let gate = Arc::new(AdmissionGate::new(1));
    let first = gate.acquire(&CancelToken::new()).unwrap();
    let (sent, received) = mpsc::channel();
    let cancel = CancelToken::new();
    for number in 0..3 {
        let queued_gate = gate.clone();
        let sender = sent.clone();
        let token = if number == 1 {
            cancel.clone()
        } else {
            CancelToken::new()
        };
        thread::spawn(move || {
            let acquired = queued_gate.acquire(&token);
            sender.send((number, acquired.is_ok())).unwrap();
        });
        wait_for_queue(&gate, number + 1);
    }
    cancel.cancel();
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (1, false)
    );
    assert_eq!(gate.queued(), 2);
    drop(first);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (0, true)
    );
    assert_eq!(
        received.recv_timeout(Duration::from_secs(2)).unwrap(),
        (2, true)
    );
}

#[test]
fn cloud_admission_caps_independent_gateway_instances_globally() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let active_handler = active.clone();
    let peak_handler = peak.clone();
    let provider = MockProvider::spawn(move |_, _| {
        let now = active_handler.fetch_add(1, Ordering::SeqCst) + 1;
        peak_handler.fetch_max(now, Ordering::SeqCst);
        thread::sleep(Duration::from_millis(120));
        active_handler.fetch_sub(1, Ordering::SeqCst);
        raw(completion("ok", &[], 10))
    });
    let clients: Vec<OpenAiCompatibleGateway> =
        (0..8).map(|_| gateway(&provider.base_url, false)).collect();
    let handles: Vec<_> = clients
        .into_iter()
        .map(|client| {
            thread::spawn(move || {
                client.complete_once(&ModelRequest::default(), &CancelToken::new())
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap().unwrap();
    }
    assert_eq!(provider.hits(), 8);
    assert!(peak.load(Ordering::SeqCst) <= CLOUD_MAX_IN_FLIGHT);
}

#[test]
fn waiting_cloud_request_can_cancel_without_taking_a_slot() {
    let gate = Arc::new(AdmissionGate::new(CLOUD_MAX_IN_FLIGHT));
    let occupied: Vec<_> = (0..CLOUD_MAX_IN_FLIGHT)
        .map(|_| gate.acquire(&CancelToken::new()).unwrap())
        .collect();
    let token = CancelToken::new();
    let waiting = token.clone();
    let queued_gate = gate.clone();
    let handle = thread::spawn(move || queued_gate.acquire(&waiting).map(|_| ()));
    wait_for_queue(&gate, 1);
    token.cancel();
    assert_eq!(handle.join().unwrap(), Err(ModelError::Cancelled));
    assert_eq!(gate.queued(), 0);
    drop(occupied);
}

// Production keeps one admission domain per deployment kind for the whole
// process: a different provider endpoint must never buy extra capacity.
// Only the in-process test harness scopes domains by fake-provider origin.
#[test]
fn production_admission_domain_is_process_wide_and_test_domain_is_per_provider_origin() {
    for local in [true, false] {
        let a = admission_domain(local, "http://127.0.0.1:4100/v1/chat/completions", false);
        let b = admission_domain(
            local,
            "https://api.other.example/v9/chat/completions",
            false,
        );
        assert_eq!(a, b, "production admission must ignore the endpoint");
        assert_ne!(
            a,
            admission_domain(!local, "http://127.0.0.1:4100/v1", false)
        );
    }
    let base = admission_domain(true, "http://127.0.0.1:4100/v1", true);
    assert_eq!(
        base,
        admission_domain(true, "HTTP://127.0.0.1:4100/v1/chat/completions", true)
    );
    assert_eq!(base, admission_domain(true, "http://127.0.0.1:4100", true));
    assert_ne!(
        base,
        admission_domain(true, "http://127.0.0.1:4101/v1", true)
    );
    assert_ne!(
        base,
        admission_domain(false, "http://127.0.0.1:4100/v1", true)
    );
}

// One test's held permit (or a slow fake provider) cannot starve another
// test's provider, and each scoped gate keeps the production capacity.
#[test]
fn scoped_admission_gates_isolate_held_permits_between_provider_origins() {
    let occupied_local = MockProvider::spawn(|_, _| raw(completion("unused", &[], 1)));
    let free_local = MockProvider::spawn(|_, _| raw(completion("ok", &[], 1)));
    let held = gate_for(
        true,
        &format!("{}/chat/completions", occupied_local.base_url),
    );
    let permit = held.acquire(&CancelToken::new()).unwrap();
    assert!(std::ptr::eq(held, gate_for(true, &occupied_local.base_url)));
    assert!(!std::ptr::eq(held, gate_for(true, &free_local.base_url)));
    let started = Instant::now();
    gateway(&free_local.base_url, true)
        .complete_once(&ModelRequest::default(), &CancelToken::new())
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(free_local.hits(), 1);
    // The occupied provider's own requests still queue behind its single permit.
    let token = CancelToken::new();
    let waiting = token.clone();
    let client = gateway(&occupied_local.base_url, true);
    let handle = thread::spawn(move || client.complete_once(&ModelRequest::default(), &waiting));
    wait_for_queue(held, 1);
    token.cancel();
    assert!(handle.join().unwrap().is_err());
    assert_eq!((held.queued(), occupied_local.hits()), (0, 0));
    drop(permit);
    let cloud = gate_for(false, &free_local.base_url);
    let permits: Vec<_> = (0..CLOUD_MAX_IN_FLIGHT)
        .map(|_| cloud.acquire(&CancelToken::new()).unwrap())
        .collect();
    let token = CancelToken::new();
    let waiting = token.clone();
    let handle = thread::spawn(move || cloud.acquire(&waiting).map(|_| ()));
    wait_for_queue(cloud, 1);
    token.cancel();
    assert_eq!(handle.join().unwrap(), Err(ModelError::Cancelled));
    drop(permits);
}
