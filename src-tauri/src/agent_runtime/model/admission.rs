//! Process-wide model request admission. Queue order is FIFO; a waiting stop
//! removes its ticket without issuing a provider request or consuming budget.
use super::gateway::{CancelToken, ModelError};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Condvar, Mutex, OnceLock},
    time::Duration,
};

pub const CLOUD_MAX_IN_FLIGHT: usize = 3;
const CANCEL_CHECK_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct AdmissionState {
    active: usize,
    next_ticket: u64,
    waiting: VecDeque<u64>,
}

pub struct AdmissionGate {
    capacity: usize,
    state: Mutex<AdmissionState>,
    changed: Condvar,
}

pub struct AdmissionPermit<'a>(&'a AdmissionGate);

impl AdmissionGate {
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0);
        Self {
            capacity,
            state: Mutex::new(AdmissionState::default()),
            changed: Condvar::new(),
        }
    }

    pub fn acquire(&self, cancel: &CancelToken) -> Result<AdmissionPermit<'_>, ModelError> {
        if cancel.is_cancelled() {
            return Err(ModelError::Cancelled);
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ticket = state.next_ticket;
        state.next_ticket = state.next_ticket.wrapping_add(1);
        state.waiting.push_back(ticket);
        loop {
            // Do not call the external cancellation checker while holding the
            // queue lock: it may need a database lock held by another waiter.
            drop(state);
            let cancelled = cancel.is_cancelled();
            state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if cancelled {
                state.waiting.retain(|queued| *queued != ticket);
                self.changed.notify_all();
                return Err(ModelError::Cancelled);
            }
            if state.active < self.capacity && state.waiting.front() == Some(&ticket) {
                state.waiting.pop_front();
                state.active += 1;
                return Ok(AdmissionPermit(self));
            }
            state = self
                .changed
                .wait_timeout(state, CANCEL_CHECK_INTERVAL)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .waiting
            .len()
    }
}

impl Drop for AdmissionPermit<'_> {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.active -= 1;
        self.0.changed.notify_all();
    }
}

static LOCAL: OnceLock<AdmissionGate> = OnceLock::new();
static CLOUD: OnceLock<AdmissionGate> = OnceLock::new();

fn capacity(local: bool) -> usize {
    if local {
        1
    } else {
        CLOUD_MAX_IN_FLIGHT
    }
}

/// The production singleton for one deployment kind.
fn process_gate(local: bool) -> &'static AdmissionGate {
    if local {
        LOCAL.get_or_init(|| AdmissionGate::new(capacity(true)))
    } else {
        CLOUD.get_or_init(|| AdmissionGate::new(capacity(false)))
    }
}

/// Key of one admission domain. `origin` is `None` for the process-wide
/// production domain and `Some(scheme://authority)` for a per-provider one.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AdmissionDomain {
    local: bool,
    origin: Option<String>,
}

/// Production runs exactly one app instance per process, so admission is one
/// domain per deployment kind and the endpoint is ignored. The unit-test
/// harness instead hosts many independent app instances in one process, each
/// talking to its own loopback fake provider; sharing the production singleton
/// there let one test's held or slow permit starve unrelated tests and made
/// exact queue assertions observe foreign waiters. Under `cfg(test)` the domain
/// is therefore also scoped by provider origin; inside one domain capacity,
/// FIFO order and cancellation are the unchanged production gate.
const PER_PROVIDER_ORIGIN: bool = cfg!(test);

pub(crate) fn admission_domain(
    local: bool,
    endpoint: &str,
    per_provider_origin: bool,
) -> AdmissionDomain {
    AdmissionDomain {
        local,
        origin: per_provider_origin.then(|| provider_origin(endpoint)),
    }
}

fn provider_origin(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    let (scheme, rest) = trimmed.split_once("://").unwrap_or(("", trimmed));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    format!(
        "{}://{}",
        scheme.to_ascii_lowercase(),
        authority.to_ascii_lowercase()
    )
}

fn scoped_gate(domain: AdmissionDomain) -> &'static AdmissionGate {
    static SCOPED: OnceLock<Mutex<HashMap<AdmissionDomain, &'static AdmissionGate>>> =
        OnceLock::new();
    let mut gates = SCOPED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let local = domain.local;
    gates
        .entry(domain)
        .or_insert_with(|| Box::leak(Box::new(AdmissionGate::new(capacity(local)))))
}

/// The admission gate a request to `endpoint` must pass.
pub fn gate_for(local: bool, endpoint: &str) -> &'static AdmissionGate {
    let domain = admission_domain(local, endpoint, PER_PROVIDER_ORIGIN);
    if domain.origin.is_none() {
        return process_gate(local);
    }
    scoped_gate(domain)
}
