//! One cancellable HTTP transport for every model request (Phase 2 §6.2).
//!
//! The previous shape ran the request on a detached blocking thread: a cancel
//! stopped the *waiting*, never the request, so a cloud generation could keep
//! running for minutes after the UI already said "stopped". Here the request
//! future itself is dropped, which drops the response body and its connection,
//! and the caller only returns once the worker that owned them has finished.
//! That is what lets the loop write a terminal event after the transport has
//! really settled.

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

/// How long a cancel may take to be noticed. The request is not polled during
/// that window; only the predicate runs.
pub const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Who ended the request: our own cancellation token, or the peer we were
/// serving going away first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelInitiator {
    /// Our token fired (pause, stop, task teardown).
    Local,
    /// The downstream connection was already gone when the next byte arrived.
    Peer,
}

/// When cancellation was asked for and when the transport was really closed.
/// Both are persisted so the UI can say whether a stop completed (§6.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CancelObservation {
    pub initiator: CancelInitiator,
    pub cancel_requested_at: String,
    pub transport_closed_at: String,
    /// How long the cancel took to settle, measured in the transport itself.
    pub settle_millis: i64,
}

/// A progress event so a proxy can stream what it receives instead of waiting
/// for the whole body. Returning `false` means "the peer left"; the transport
/// then drops the request and reports a cancellation.
pub enum TransportEvent<'a> {
    Head {
        status: u16,
        headers: &'a [(String, String)],
    },
    Body(&'a [u8]),
}

#[derive(Clone, Debug)]
pub struct TransportReply {
    pub status: u16,
    pub body: Vec<u8>,
}

#[derive(Clone, Debug)]
pub enum TransportError {
    /// The client could not be built (invalid proxy or TLS configuration).
    Setup(String),
    Network(String),
    Timeout(String),
    /// The response could not be read to the end.
    Interrupted(String),
    /// Cancellation won: the future, the body and the connection were dropped.
    Cancelled(CancelObservation),
}

#[derive(Clone, Debug)]
pub struct TransportRequest {
    pub endpoint: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub connect_timeout: Duration,
    /// `None` means no overall timeout, which is what a long local generation
    /// needs; cancellation is what bounds it instead.
    pub total_timeout: Option<Duration>,
    pub proxy: Option<String>,
}

impl TransportRequest {
    pub fn post(endpoint: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            endpoint: endpoint.into(),
            method: "POST".to_string(),
            headers: Vec::new(),
            body,
            connect_timeout: Duration::from_secs(15),
            total_timeout: None,
            proxy: None,
        }
    }

    pub fn header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

type CancelPredicate = Arc<dyn Fn() -> bool + Send + Sync>;
type Sink = Box<dyn FnMut(TransportEvent<'_>) -> bool + Send>;

/// Send one request and read it to the end, or report the cancel that stopped it.
///
/// The runtime lives on its own short-lived thread so this never nests inside a
/// Tauri runtime, and the join is what makes the stop deterministic: no request
/// handle outlives this call.
pub fn send(
    request: TransportRequest,
    cancel: CancelPredicate,
    sink: Sink,
) -> Result<TransportReply, TransportError> {
    send_observed(request, cancel, sink, Box::new(|| {}))
}

/// An identity-bound diagnostic can observe transport start without seeing I/O.
pub(crate) fn send_observed(
    request: TransportRequest,
    cancel: CancelPredicate,
    sink: Sink,
    started: Box<dyn FnOnce() + Send>,
) -> Result<TransportReply, TransportError> {
    let worker = thread::spawn(move || block_send(request, cancel, sink, started));
    worker
        .join()
        .unwrap_or_else(|_| Err(TransportError::Network("模型请求线程意外结束".to_string())))
}

fn block_send(
    request: TransportRequest,
    cancel: CancelPredicate,
    sink: Sink,
    observe_started: Box<dyn FnOnce() + Send>,
) -> Result<TransportReply, TransportError> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| TransportError::Setup(error.to_string()))?;
    let started = Instant::now();
    let outcome = runtime.block_on(async move { drive(request, cancel, sink, observe_started).await });
    // Dropping the runtime is what cancels the connection task, so the socket is
    // closed by the time the settle timestamp is taken.
    drop(runtime);
    match outcome {
        Err(TransportError::Cancelled(mut observation))
            if observation.transport_closed_at.is_empty() =>
        {
            observation.transport_closed_at = now_stamp();
            observation.settle_millis = started.elapsed().as_millis().min(i64::MAX as u128) as i64;
            Err(TransportError::Cancelled(observation))
        }
        other => other,
    }
}

async fn drive(
    request: TransportRequest,
    cancel: CancelPredicate,
    mut sink: Sink,
    observe_started: Box<dyn FnOnce() + Send>,
) -> Result<TransportReply, TransportError> {
    let mut builder = reqwest::Client::builder().connect_timeout(request.connect_timeout);
    if let Some(timeout) = request.total_timeout {
        builder = builder.timeout(timeout);
    }
    if let Some(proxy) = request.proxy.as_deref() {
        let proxy = reqwest::Proxy::all(proxy)
            .map_err(|_| TransportError::Setup("代理地址格式无效".to_string()))?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|error| TransportError::Setup(error.to_string()))?;
    let method = reqwest::Method::from_bytes(request.method.as_bytes())
        .map_err(|error| TransportError::Setup(error.to_string()))?;
    let mut call = client.request(method, request.endpoint.as_str());
    for (name, value) in &request.headers {
        call = call.header(name.as_str(), value.as_str());
    }
    let call = call.body(request.body);
    let fetch = async move {
        // Constant only: start is durable before the actual send future is polled.
        // It does not prove the provider accepted or executed this request.
        observe_started();
        let response = call.send().await.map_err(describe_failure)?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(name, value)| {
                (
                    name.as_str().to_ascii_lowercase(),
                    value.to_str().unwrap_or_default().to_string(),
                )
            })
            .collect::<Vec<_>>();
        if !sink(TransportEvent::Head {
            status,
            headers: &headers,
        }) {
            return Err(cancelled(CancelInitiator::Peer));
        }
        let mut body = Vec::new();
        let mut response = response;
        loop {
            let Some(chunk) = response.chunk().await.map_err(describe_failure)? else {
                break;
            };
            if !sink(TransportEvent::Body(&chunk)) {
                return Err(cancelled(CancelInitiator::Peer));
            }
            body.extend_from_slice(&chunk);
        }
        Ok(TransportReply { status, body })
    };
    settle(fetch, wait_for_cancel(&cancel)).await
    // Falling out of `select!` is what drops `fetch`, its response body and the
    // connection underneath them.
}

/// Race the request against cancellation. `biased` decides only a tie: when
/// the reply has already been read to the end by the time a cancel probe also
/// fires in the same poll, the known (billed) reply wins instead of a coin
/// flip turning it into an unknown outcome. A reply still on the wire keeps
/// losing to cancellation, and callers re-check authority after receipt.
pub(crate) async fn settle(
    fetch: impl std::future::Future<Output = Result<TransportReply, TransportError>>,
    cancel: impl std::future::Future<Output = ()>,
) -> Result<TransportReply, TransportError> {
    tokio::select! {
        biased;
        outcome = fetch => outcome,
        _ = cancel => Err(cancelled(CancelInitiator::Local)),
    }
}

fn cancelled(initiator: CancelInitiator) -> TransportError {
    TransportError::Cancelled(CancelObservation {
        initiator,
        cancel_requested_at: now_stamp(),
        transport_closed_at: String::new(),
        settle_millis: 0,
    })
}

fn describe_failure(error: reqwest::Error) -> TransportError {
    if error.is_timeout() {
        TransportError::Timeout(error.to_string())
    } else if error.is_body() || error.is_decode() {
        TransportError::Interrupted(error.to_string())
    } else {
        TransportError::Network(error.to_string())
    }
}

async fn wait_for_cancel(cancel: &CancelPredicate) {
    // The caller checked the token before opening the request. Waiting a full
    // interval before the first probe keeps a response that arrives immediately
    // from racing with its own cancellation check.
    let mut ticker = tokio::time::interval(CANCEL_POLL_INTERVAL);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    ticker.tick().await;
    loop {
        ticker.tick().await;
        if cancel() {
            return;
        }
    }
}

fn now_stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
