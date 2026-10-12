use super::log::Event;
use crate::log_framing::{Framer, Record};
use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};
const RECORD_BYTES: usize = 16 * 1024;
const QUEUE_CAPACITY: usize = 64;
/// Bounded time a stopped reader keeps draining for EOF after the owner killed
/// the process group. Callers wait 250 ms for the drained capture.
pub(crate) const STOP_DRAIN_GRACE: Duration = Duration::from_millis(100);

#[derive(Default)]
struct DropStats {
    stdout: AtomicUsize,
    stderr: AtomicUsize,
}

pub(crate) struct Channel {
    sender: mpsc::SyncSender<Event>,
    receiver: mpsc::Receiver<Event>,
    dropped: Arc<DropStats>,
}
impl Channel {
    pub(crate) fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        Self {
            sender,
            receiver,
            dropped: Arc::new(DropStats::default()),
        }
    }
    pub(crate) fn pipe(&self, stream: &'static str) -> Pipe {
        Pipe {
            sender: self.sender.clone(),
            dropped: Arc::clone(&self.dropped),
            stream,
            sequence: 0,
            framer: Framer::new(RECORD_BYTES),
        }
    }
    pub(crate) fn deliver(
        &self,
        observe: &mut dyn FnMut(&Event) -> Result<(), String>,
        error: &mut Option<String>,
        final_drain: bool,
    ) {
        let started = Instant::now();
        for _ in 0..QUEUE_CAPACITY {
            let Ok(event) = self.receiver.try_recv() else {
                break;
            };
            if event.gap {
                error.get_or_insert_with(|| "native_process_log_record_omitted".into());
            }
            if observe(&event).is_err() {
                error.get_or_insert_with(|| "native_process_log_persistence_failed".into());
            }
            if !final_drain && started.elapsed() >= Duration::from_millis(5) {
                break;
            }
        }
    }
    pub(crate) fn finish(
        &self,
        observe: &mut dyn FnMut(&Event) -> Result<(), String>,
        error: &mut Option<String>,
    ) {
        self.deliver(observe, error, true);
        let stdout = self.dropped.stdout.load(Ordering::Acquire);
        let stderr = self.dropped.stderr.load(Ordering::Acquire);
        if stdout > 0 || stderr > 0 {
            error.get_or_insert_with(|| "native_process_log_queue_gap".into());
            // One aggregate marker fits even when the event queue was full.
            let _ = observe(&Event {
                stream: "gap",
                stream_sequence: 1,
                message: format!(
                    "Native output queue omitted records: stdout={stdout} stderr={stderr}"
                ),
                gap: true,
            });
        }
    }
}

pub(crate) struct Pipe {
    sender: mpsc::SyncSender<Event>,
    dropped: Arc<DropStats>,
    stream: &'static str,
    sequence: i64,
    framer: Framer,
}
impl Pipe {
    fn emit(
        sender: &mpsc::SyncSender<Event>,
        dropped: &DropStats,
        stream: &'static str,
        sequence: &mut i64,
        record: Record,
    ) -> bool {
        *sequence = sequence.saturating_add(1);
        let (mut message, mut gap) = match record {
            Record::Bytes(bytes) => (
                crate::agent_runtime::secrets::redact_text_with(
                    &String::from_utf8_lossy(&bytes),
                    None,
                ),
                false,
            ),
            Record::Oversized => (
                "[Native output record exceeded 16384 bytes; omitted whole]".into(),
                true,
            ),
        };
        if message.len() > 64 * 1024 {
            message =
                "[Native redacted output exceeded bounded message size; omitted whole]".into();
            gap = true;
        }
        if sender
            .try_send(Event {
                stream,
                stream_sequence: *sequence,
                message,
                gap,
            })
            .is_err()
        {
            let count = if stream == "stdout" {
                &dropped.stdout
            } else {
                &dropped.stderr
            };
            count.fetch_add(1, Ordering::Relaxed);
        }
        // Dropped notifications must never block the raw pipe drain or cancellation.
        true
    }
    fn feed(&mut self, bytes: &[u8]) {
        let (sender, dropped, stream, sequence) =
            (&self.sender, &self.dropped, self.stream, &mut self.sequence);
        self.framer.feed(bytes, &mut |record| {
            Self::emit(sender, dropped, stream, sequence, record)
        });
    }
    fn failed(&mut self) {
        self.framer.abandon_pending();
        self.sequence = self.sequence.saturating_add(1);
        let message = format!(
            "Native {} output capture incomplete; partial record omitted",
            self.stream
        );
        if self
            .sender
            .try_send(Event {
                stream: self.stream,
                stream_sequence: self.sequence,
                message,
                gap: true,
            })
            .is_err()
        {
            let count = if self.stream == "stdout" {
                &self.dropped.stdout
            } else {
                &self.dropped.stderr
            };
            count.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn finish(&mut self) {
        let (sender, dropped, stream, sequence) =
            (&self.sender, &self.dropped, self.stream, &mut self.sequence);
        self.framer
            .finish(&mut |record| Self::emit(sender, dropped, stream, sequence, record));
    }
}

/// Original raw capture stays byte-exact and separately capped. The observer
/// receives complete redacted records even after that raw capture reaches its cap.
pub(crate) fn pump(
    mut pipe: impl Read,
    limit: usize,
    stop: Arc<AtomicBool>,
    sender: mpsc::Sender<(Vec<u8>, bool)>,
    mut observer: Option<Pipe>,
) {
    let mut buffer = [0u8; 16 * 1024];
    let mut collected = Vec::new();
    let mut truncated = false;
    let mut capture_failed = false;
    let mut stopping_at = None;
    // Whether the latest read found the pipe open but empty. A stop grace that
    // expires while idle means a write end is still held (capture incomplete);
    // expiring while bytes still flow additionally means output was cut.
    let mut idle = false;
    loop {
        if stop.load(Ordering::Acquire) {
            let since = stopping_at.get_or_insert_with(Instant::now);
            if since.elapsed() >= STOP_DRAIN_GRACE {
                truncated |= !idle;
                capture_failed = true;
                break;
            }
        }
        let read = match pipe.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                // SIGKILL of the process group is asynchronous: a just-killed
                // descendant still owns its inherited write end for a moment
                // after `stop` is raised. Keep polling within the same bounded
                // grace so EOF arriving then is a clean finish, not a spurious
                // `native_process_log_record_omitted`; a write end held past the
                // grace still fails the capture above.
                idle = true;
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            Err(_) => {
                truncated = true;
                capture_failed = true;
                break;
            }
        };
        if read == 0 {
            break;
        }
        idle = false;
        if let Some(observer) = &mut observer {
            observer.feed(&buffer[..read]);
        }
        let room = limit.saturating_sub(collected.len());
        if room == 0 {
            truncated = true;
            continue;
        }
        let take = room.min(read);
        collected.extend_from_slice(&buffer[..take]);
        if take < read {
            truncated = true;
        }
    }
    if let Some(observer) = &mut observer {
        if capture_failed {
            observer.failed();
        } else {
            observer.finish();
        }
    }
    let _ = sender.send((collected, truncated));
}

#[cfg(all(test, unix))]
mod stop_drain_tests {
    use super::*;
    include!("process_output_stop_drain_tests.rs");
}
