// Regression for the parallel-only `native_process_log_record_omitted` flake:
// process-group SIGKILL is asynchronous, so a killed descendant may still own
// its inherited write end for a few microseconds after the owner raises `stop`.
use std::io::Write;

fn stopped_pump_with_writer_released_after(
    hold: Duration,
) -> (Vec<u8>, bool, Option<String>, Vec<Event>, Duration) {
    let (reader, mut writer) = std::io::pipe().unwrap();
    super::super::interruptible(&reader).unwrap();
    // Unterminated tail: only a clean EOF may publish it as a complete record.
    writer.write_all(b"safe-eof-tail").unwrap();
    let channel = Channel::new();
    let stop = Arc::new(AtomicBool::new(true));
    let (sender, receiver) = mpsc::channel();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(hold);
        drop(writer);
    });
    let started = Instant::now();
    pump(reader, 1024, stop, sender, Some(channel.pipe("stderr")));
    let elapsed = started.elapsed();
    let (collected, truncated) = receiver.recv().unwrap();
    releaser.join().unwrap();
    let mut events = Vec::new();
    let mut error = None;
    channel.finish(
        &mut |event: &Event| {
            events.push(Event {
                stream: event.stream,
                stream_sequence: event.stream_sequence,
                message: event.message.clone(),
                gap: event.gap,
            });
            Ok(())
        },
        &mut error,
    );
    (collected, truncated, error, events, elapsed)
}

#[test]
fn stopped_reader_accepts_eof_that_arrives_within_the_kill_grace() {
    let (collected, truncated, error, events, _) =
        stopped_pump_with_writer_released_after(Duration::from_millis(20));
    assert_eq!(collected, b"safe-eof-tail");
    assert!(!truncated);
    assert_eq!(
        error, None,
        "late-but-bounded EOF must not be reported as an omitted record"
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].message, "safe-eof-tail");
    assert!(!events[0].gap);
}

#[test]
fn stopped_reader_still_fails_a_write_end_held_past_the_kill_grace() {
    let (collected, truncated, error, events, elapsed) =
        stopped_pump_with_writer_released_after(Duration::from_millis(600));
    // Bytes read before the stop stay exact, but the unterminated tail is never
    // published as complete and the capture is marked incomplete.
    assert_eq!(collected, b"safe-eof-tail");
    assert!(
        !truncated,
        "an idle held-open pipe is incomplete, not cut output"
    );
    assert_eq!(error.as_deref(), Some("native_process_log_record_omitted"));
    assert!(events.iter().all(|event| event.message != "safe-eof-tail"));
    assert!(events.iter().any(|event| event.gap));
    assert!(
        elapsed >= STOP_DRAIN_GRACE,
        "gave up before the grace: {elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_millis(250),
        "a held write end must not stall past the callers' 250 ms drain wait: {elapsed:?}"
    );
}
