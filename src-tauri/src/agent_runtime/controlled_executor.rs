//! Shared multi-round control, not authority. Only adapters owning original
//! runtime bindings may transport, admit, execute and persist actual effects.
use serde_json::{json, Value};
use std::collections::HashSet;

pub(crate) struct Limits {
    pub(crate) rounds: usize,
    pub(crate) tools: Option<usize>,
    pub(crate) calls_per_round: Option<usize>,
    pub(crate) argument_bytes: Option<usize>,
}
pub(crate) struct RawToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: String,
}
pub(crate) struct Received<P> {
    pub(crate) paid: P,
    pub(crate) text: String,
    pub(crate) calls: Vec<RawToolCall>,
}
#[derive(Debug)]
pub(crate) enum Stop {
    Cancelled,
    Limit(&'static str),
    Unknown(String),
    Refused(String),
    Persistence(String),
    PaidAbort {
        original: Box<Stop>,
        settlement: Box<Stop>,
    },
}
pub(crate) enum TextPolicy {
    Continue(Vec<Value>),
    Complete,
}
pub(crate) enum ToolPolicy {
    Continue,
    Complete,
}
pub(crate) struct ToolEffect<E> {
    pub(crate) original: E,
    pub(crate) observation: Value,
    pub(crate) policy: ToolPolicy,
}
// This crate-private trait grants nothing. Implementations must retain their
// original typed scope/fee witnesses; the core cannot fabricate either token.
pub(crate) trait Adapter {
    type RoundPermit;
    type PaidResponse;
    type ToolPermit;
    type ToolOriginal;
    type Audit;
    type Output;
    fn registered_tools(&self) -> &[String];
    // Pure live/cancel recheck, including deterministic rejection observations.
    fn check_continuation(&mut self) -> Result<(), Stop>;
    fn before_round(&mut self, round: usize) -> Result<Self::RoundPermit, Stop>;
    // Exactly one physical model call. Unknown response is Stop::Unknown;
    // neither the core nor a new history entry makes it retryable.
    fn model_once(
        &mut self,
        permit: Self::RoundPermit,
        history: &[Value],
    ) -> Result<Received<Self::PaidResponse>, Stop>;
    fn record_paid_response(&mut self, paid: &Self::PaidResponse, round: usize)
        -> Result<(), Stop>;
    // Financial cleanup only: no retry, model call, tool or publication. The
    // original paid witness survives every failure after model_once succeeds.
    fn abort_paid_response(&mut self, paid: &Self::PaidResponse, stop: &Stop) -> Result<(), Stop> {
        let _ = (paid, stop);
        Ok(())
    }
    // Default preserves whole-group identity rejection. A typed adapter may
    // delegate to its original ordered semantic verifier without adding grants.
    fn validate_call_group(
        &mut self,
        paid: &Self::PaidResponse,
        calls: &[RawToolCall],
    ) -> Result<(), Stop> {
        let _ = paid;
        let mut ids = HashSet::new();
        for call in calls {
            if call.id.is_empty() || !ids.insert(call.id.as_str()) {
                return Err(Stop::Refused(
                    "controlled_executor_call_identity_invalid".into(),
                ));
            }
        }
        Ok(())
    }
    fn text_policy(&mut self, paid: &Self::PaidResponse, text: &str) -> Result<TextPolicy, Stop>;
    fn validate_arguments(&self, name: &str, arguments: &Value) -> Result<(), String>;
    fn before_tool(
        &mut self,
        paid: &Self::PaidResponse,
        id: &str,
        name: &str,
        arguments: &Value,
    ) -> Result<Self::ToolPermit, Stop>;
    fn execute_tool(
        &mut self,
        permit: Self::ToolPermit,
        name: &str,
        arguments: &Value,
    ) -> Result<ToolEffect<Self::ToolOriginal>, Stop>;
    // Audit before exposing any observation to the next model. A normal
    // deterministic tool error may be a Continue effect; unknown must stop.
    fn audit_tool(
        &mut self,
        paid: &Self::PaidResponse,
        id: &str,
        name: &str,
        arguments: &Value,
        effect: &ToolEffect<Self::ToolOriginal>,
    ) -> Result<Self::Audit, Stop>;
    fn audit_rejection(
        &mut self,
        paid: &Self::PaidResponse,
        id: &str,
        name: &str,
        raw_arguments: &str,
        rejection: &str,
    ) -> Result<(Self::Audit, Value), Stop>;
    // Presentation only, after the original audit; Web may retain its original
    // redacted name/artifact fields and truncation without moving iteration.
    fn tool_message(&self, id: &str, name: &str, observation: &Value) -> Value {
        let _ = name;
        json!({"role":"tool","tool_call_id":id,"content":observation.to_string()})
    }
    fn checkpoint(&mut self, history: &[Value], audits: &[Self::Audit]) -> Result<(), Stop>;
    fn after_tools(
        &mut self,
        paid: &Self::PaidResponse,
        history: &[Value],
    ) -> Result<TextPolicy, Stop>;
    fn finish(
        &mut self,
        paid: &Self::PaidResponse,
        history: &[Value],
    ) -> Result<Self::Output, Stop>;
}

pub(crate) fn run<A: Adapter>(
    adapter: &mut A,
    limits: &Limits,
    mut history: Vec<Value>,
) -> Result<A::Output, Stop> {
    if limits.rounds == 0 || limits.calls_per_round == Some(0) || limits.argument_bytes == Some(0) {
        return Err(Stop::Refused("controlled_executor_limits_invalid".into()));
    }
    let registered = adapter
        .registered_tools()
        .iter()
        .cloned()
        .collect::<HashSet<_>>();
    if registered.len() != adapter.registered_tools().len()
        || registered.iter().any(|name| name.is_empty())
    {
        return Err(Stop::Refused("controlled_executor_registry_invalid".into()));
    }
    let mut tools_used = 0usize;
    for round in 1..=limits.rounds {
        adapter.check_continuation()?;
        let permit = adapter.before_round(round)?;
        let received = adapter.model_once(permit, &history)?;
        let paid_round = (|| -> Result<Option<A::Output>, Stop> {
            adapter.record_paid_response(&received.paid, round)?;
            // Hard malformed-response boundaries occur after accounting. No tool
            // from this response runs if its group is oversized or IDs conflict.
            if registered.is_empty() && !received.calls.is_empty() {
                return Err(Stop::Refused(
                    "controlled_executor_readonly_tools_denied".into(),
                ));
            }
            if limits
                .calls_per_round
                .is_some_and(|bound| received.calls.len() > bound)
            {
                return Err(Stop::Limit("controlled_executor_call_group_limit"));
            }
            // Provider fallback IDs may recur in another paid model call. Audit
            // identity comes from that original call and its invocation receipt.
            adapter.validate_call_group(&received.paid, &received.calls)?;
            history.push(json!({"role":"assistant","content":received.text,"tool_calls":received.calls.iter().map(|c|json!({"id":c.id,"type":"function","function":{"name":c.name,"arguments":c.arguments}})).collect::<Vec<_>>()}));
            if received.calls.is_empty() {
                match adapter.text_policy(&received.paid, &received.text)? {
                    TextPolicy::Continue(messages) => {
                        history.extend(messages);
                        adapter.checkpoint(&history, &[])?;
                        return Ok(None);
                    }
                    TextPolicy::Complete => {
                        adapter.checkpoint(&history, &[])?;
                        return adapter.finish(&received.paid, &history).map(Some);
                    }
                }
            }
            let mut audits = Vec::new();
            let mut complete = false;
            for call in &received.calls {
                adapter.check_continuation()?;
                // Counts all proposed calls, including deterministic invalid ones,
                // so repeated bad arguments cannot bypass the execution bound.
                if limits.tools.is_some_and(|bound| tools_used >= bound) {
                    return Err(Stop::Limit("controlled_executor_tool_limit"));
                }
                tools_used = tools_used
                    .checked_add(1)
                    .ok_or(Stop::Limit("controlled_executor_tool_limit"))?;
                let parsed = if !registered.contains(&call.name) {
                    Err("controlled_executor_tool_not_registered".to_string())
                } else if limits
                    .argument_bytes
                    .is_some_and(|bound| call.arguments.len() > bound)
                {
                    Err("controlled_executor_arguments_too_large".to_string())
                } else {
                    serde_json::from_str::<Value>(&call.arguments)
                        .map_err(|_| "controlled_executor_arguments_invalid_json".to_string())
                        .and_then(|v| {
                            if v.is_object() {
                                adapter.validate_arguments(&call.name, &v).map(|()| v)
                            } else {
                                Err("controlled_executor_arguments_object_required".into())
                            }
                        })
                };
                let observation = match parsed {
                    Err(code) => {
                        let (audit, observation) = adapter.audit_rejection(
                            &received.paid,
                            &call.id,
                            &call.name,
                            &call.arguments,
                            &code,
                        )?;
                        audits.push(audit);
                        observation
                    }
                    Ok(arguments) => {
                        let permit = adapter.before_tool(
                            &received.paid,
                            &call.id,
                            &call.name,
                            &arguments,
                        )?;
                        let effect = adapter.execute_tool(permit, &call.name, &arguments)?;
                        audits.push(adapter.audit_tool(
                            &received.paid,
                            &call.id,
                            &call.name,
                            &arguments,
                            &effect,
                        )?);
                        complete = matches!(effect.policy, ToolPolicy::Complete);
                        effect.observation
                    }
                };
                history.push(adapter.tool_message(&call.id, &call.name, &observation));
                adapter.checkpoint(&history, &audits)?;
                audits.clear();
                if complete {
                    return adapter.finish(&received.paid, &history).map(Some);
                }
            }
            match adapter.after_tools(&received.paid, &history)? {
                TextPolicy::Continue(messages) => {
                    history.extend(messages);
                    adapter.checkpoint(&history, &[])?;
                    Ok(None)
                }
                TextPolicy::Complete => {
                    adapter.checkpoint(&history, &[])?;
                    adapter.finish(&received.paid, &history).map(Some)
                }
            }
        })();
        match paid_round {
            Ok(Some(output)) => return Ok(output),
            Ok(None) => {}
            Err(original) => {
                return match adapter.abort_paid_response(&received.paid, &original) {
                    Ok(()) => Err(original),
                    Err(settlement) => Err(Stop::PaidAbort {
                        original: Box::new(original),
                        settlement: Box::new(settlement),
                    }),
                };
            }
        }
    }
    Err(Stop::Limit("controlled_executor_round_limit"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        tools: Vec<String>,
        rounds: usize,
        paid: usize,
        executed: usize,
        audited: usize,
        cancel: bool,
        unknown: bool,
        duplicate: bool,
        empty_name: bool,
        aborted: usize,
        cancel_after_paid: bool,
        reject_audit: bool,
        abort_failure: bool,
        unknown_after_paid: bool,
    }
    impl Adapter for Fixture {
        type RoundPermit = ();
        type PaidResponse = ();
        type ToolPermit = ();
        type ToolOriginal = ();
        type Audit = ();
        type Output = usize;
        fn registered_tools(&self) -> &[String] {
            &self.tools
        }
        fn check_continuation(&mut self) -> Result<(), Stop> {
            if self.cancel {
                Err(Stop::Cancelled)
            } else {
                Ok(())
            }
        }
        fn before_round(&mut self, _: usize) -> Result<(), Stop> {
            if self.cancel {
                Err(Stop::Cancelled)
            } else {
                Ok(())
            }
        }
        fn model_once(&mut self, _: (), history: &[Value]) -> Result<Received<()>, Stop> {
            self.rounds += 1;
            if self.duplicate {
                return Ok(Received {
                    paid: (),
                    text: String::new(),
                    calls: vec![
                        RawToolCall {
                            id: "call_0".into(),
                            name: "read".into(),
                            arguments: "{}".into(),
                        },
                        RawToolCall {
                            id: "call_0".into(),
                            name: "read".into(),
                            arguments: "{}".into(),
                        },
                    ],
                });
            }
            if self.unknown {
                return Err(Stop::Unknown("original_unknown".into()));
            }
            if self.rounds == 1 {
                Ok(Received {
                    paid: (),
                    text: String::new(),
                    calls: vec![RawToolCall {
                        id: "call_0".into(),
                        name: if self.empty_name {
                            String::new()
                        } else {
                            "read".into()
                        },
                        arguments: "{".into(),
                    }],
                })
            } else {
                let original = history
                    .iter()
                    .find(|v| v["role"] == "tool")
                    .expect("audited observation before next model");
                let observation: Value =
                    serde_json::from_str(original["content"].as_str().unwrap()).unwrap();
                assert_eq!(observation["code"], "original_schema_rejection");
                assert_eq!(observation["auditMarker"], "persisted-original-observation");
                assert_eq!(observation["executed"], false);
                Ok(Received {
                    paid: (),
                    text: String::new(),
                    calls: vec![RawToolCall {
                        id: "call_0".into(),
                        name: "read".into(),
                        arguments: "{}".into(),
                    }],
                })
            }
        }
        fn record_paid_response(&mut self, _: &(), _: usize) -> Result<(), Stop> {
            self.paid += 1;
            if self.cancel_after_paid {
                self.cancel = true;
            }
            if self.unknown_after_paid {
                return Err(Stop::Unknown("original_invoice_unknown".into()));
            }
            Ok(())
        }
        fn abort_paid_response(&mut self, _: &(), _: &Stop) -> Result<(), Stop> {
            self.aborted += 1;
            if self.abort_failure {
                Err(Stop::Persistence("original_settlement_failed".into()))
            } else {
                Ok(())
            }
        }
        fn text_policy(&mut self, _: &(), _: &str) -> Result<TextPolicy, Stop> {
            Ok(TextPolicy::Continue(Vec::new()))
        }
        fn validate_arguments(&self, _: &str, _: &Value) -> Result<(), String> {
            Ok(())
        }
        fn before_tool(&mut self, _: &(), _: &str, _: &str, _: &Value) -> Result<(), Stop> {
            if self.cancel {
                Err(Stop::Cancelled)
            } else {
                Ok(())
            }
        }
        fn execute_tool(&mut self, _: (), _: &str, _: &Value) -> Result<ToolEffect<()>, Stop> {
            self.executed += 1;
            Ok(ToolEffect {
                original: (),
                observation: json!({"observed":true}),
                policy: ToolPolicy::Complete,
            })
        }
        fn audit_tool(
            &mut self,
            _: &(),
            _: &str,
            _: &str,
            _: &Value,
            effect: &ToolEffect<()>,
        ) -> Result<(), Stop> {
            let _ = &effect.original;
            self.audited += 1;
            Ok(())
        }
        fn audit_rejection(
            &mut self,
            _: &(),
            _: &str,
            _: &str,
            _: &str,
            code: &str,
        ) -> Result<((), Value), Stop> {
            self.audited += 1;
            if self.reject_audit {
                return Err(Stop::Refused("original_schema_rejection".into()));
            }
            Ok((
                (),
                json!({"code":"original_schema_rejection","sourceCode":code,"auditMarker":"persisted-original-observation","executed":false}),
            ))
        }
        fn checkpoint(&mut self, _: &[Value], _: &[()]) -> Result<(), Stop> {
            Ok(())
        }
        fn after_tools(&mut self, _: &(), _: &[Value]) -> Result<TextPolicy, Stop> {
            Ok(TextPolicy::Continue(Vec::new()))
        }
        fn finish(&mut self, _: &(), _: &[Value]) -> Result<usize, Stop> {
            Ok(self.executed)
        }
    }
    fn fixture() -> Fixture {
        Fixture {
            tools: vec!["read".into()],
            rounds: 0,
            paid: 0,
            executed: 0,
            audited: 0,
            cancel: false,
            unknown: false,
            duplicate: false,
            empty_name: false,
            aborted: 0,
            cancel_after_paid: false,
            reject_audit: false,
            abort_failure: false,
            unknown_after_paid: false,
        }
    }
    fn limits() -> Limits {
        Limits {
            rounds: 2,
            tools: Some(2),
            calls_per_round: Some(2),
            argument_bytes: Some(64),
        }
    }
    #[test]
    fn controlled_cross_round_fallback_id_and_error_observation() {
        let mut f = fixture();
        assert!(matches!(run(&mut f, &limits(), vec![]), Ok(1)));
        assert_eq!((f.rounds, f.paid, f.executed, f.audited), (2, 2, 1, 2));
    }
    #[test]
    fn controlled_cancel_prevents_model() {
        let mut f = fixture();
        f.cancel = true;
        assert!(matches!(
            run(&mut f, &limits(), vec![]),
            Err(Stop::Cancelled)
        ));
        assert_eq!(f.rounds, 0);
    }
    #[test]
    fn controlled_unknown_never_retries() {
        let mut f = fixture();
        f.unknown = true;
        assert!(matches!(
            run(&mut f, &limits(), vec![]),
            Err(Stop::Unknown(_))
        ));
        assert_eq!((f.rounds, f.paid, f.executed), (1, 0, 0));
    }
    #[test]
    fn controlled_limit_preserves_paid_rejection() {
        let mut f = fixture();
        let mut l = limits();
        l.tools = Some(1);
        assert!(matches!(
            run(&mut f, &l, vec![]),
            Err(Stop::Limit("controlled_executor_tool_limit"))
        ));
        assert_eq!((f.rounds, f.paid, f.executed, f.audited), (2, 2, 0, 1));
    }
    #[test]
    fn controlled_duplicate_id_in_one_paid_round_refuses_before_tool() {
        let mut f = fixture();
        f.duplicate = true;
        assert!(
            matches!(run(&mut f,&limits(),vec![]),Err(Stop::Refused(ref code)) if code=="controlled_executor_call_identity_invalid")
        );
        assert_eq!((f.rounds, f.paid, f.executed, f.audited), (1, 1, 0, 0));
    }
    #[test]
    fn controlled_empty_name_is_audited_and_corrected() {
        let mut f = fixture();
        f.empty_name = true;
        assert!(matches!(run(&mut f, &limits(), vec![]), Ok(1)));
        assert_eq!((f.rounds, f.paid, f.executed, f.audited), (2, 2, 1, 2));
    }
    #[test]
    fn controlled_paid_cancel_settles_once_without_tool_or_retry() {
        let mut f = fixture();
        f.cancel_after_paid = true;
        assert!(matches!(
            run(&mut f, &limits(), vec![]),
            Err(Stop::Cancelled)
        ));
        assert_eq!(
            (f.rounds, f.paid, f.aborted, f.executed, f.audited),
            (1, 1, 1, 0, 0)
        );
    }
    #[test]
    fn controlled_paid_schema_stop_settles_once_without_retry() {
        let mut f = fixture();
        f.reject_audit = true;
        assert!(
            matches!(run(&mut f, &limits(), vec![]), Err(Stop::Refused(ref c)) if c == "original_schema_rejection")
        );
        assert_eq!(
            (f.rounds, f.paid, f.aborted, f.executed, f.audited),
            (1, 1, 1, 0, 1)
        );
    }
    #[test]
    fn controlled_paid_unknown_settles_once_without_retry() {
        let mut f = fixture();
        f.unknown_after_paid = true;
        assert!(
            matches!(run(&mut f, &limits(), vec![]), Err(Stop::Unknown(ref c)) if c == "original_invoice_unknown")
        );
        assert_eq!((f.rounds, f.paid, f.aborted, f.executed), (1, 1, 1, 0));
    }
    #[test]
    fn controlled_abort_failure_preserves_original_and_settlement_errors() {
        let mut f = fixture();
        f.cancel_after_paid = true;
        f.abort_failure = true;
        match run(&mut f, &limits(), vec![]) {
            Err(Stop::PaidAbort {
                original,
                settlement,
            }) => {
                assert!(matches!(*original, Stop::Cancelled));
                assert!(
                    matches!(*settlement, Stop::Persistence(ref c) if c == "original_settlement_failed")
                );
            }
            _ => panic!("both original stop and settlement failure must survive"),
        }
        assert_eq!((f.rounds, f.paid, f.aborted, f.executed), (1, 1, 1, 0));
    }
    #[test]
    fn controlled_success_and_unpaid_unknown_do_not_abort() {
        let mut f = fixture();
        assert!(matches!(run(&mut f, &limits(), vec![]), Ok(1)));
        assert_eq!(f.aborted, 0);
        let mut f = fixture();
        f.unknown = true;
        assert!(matches!(
            run(&mut f, &limits(), vec![]),
            Err(Stop::Unknown(_))
        ));
        assert_eq!((f.rounds, f.aborted), (1, 0));
    }
}
