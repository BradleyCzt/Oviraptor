# Master 原失败名单定向复验（2026-10-08）

原整库历史终态2806通过/63失败保留。当前已编译程序固定副本SHA见 `/tmp/oviraptor-master-original-failures-20261008/inputs.json`；对原63名逐一--exact运行，无删除/ignore/预算放宽。

终态：原名单42通过、20失败、1旧名变更；旧名对应只读v2语义与新协议拒绝改写两个替代用例各通过。不要将只读v2校验当历史Root恢复执行授权。此处不是当前全库失败总数，库存2894的其余用例未测。部分测试读取实时源，开始/终态源码漂移名单见summary.json。

本次失败分组：14项原Web退出证明缺失、2项显式付费恢复合同要求、2项多物理事实Root监督容量、1项付费SDK检查点未到达、1项独立gap流程旧请求数断言。分组是实际panic分类，不自动证明每组只有一个根因。

## 20 项真实失败

1. `commands::agent_tests::coordinator_tick_actual_sigkill_recovers_original_decision_without_new_sdk`

   原日志 `15.log`，exit101，0.44秒。

```text
src/commands/agent_tests_tick_kill.rs:61:5:
actual production paid SDK checkpoint was not reached
```

2. `commands::agent_tests::coordinator_unsupported_trigger_refuses_before_another_actual_root_sdk_or_write`

   原日志 `16.log`，exit101，1.41秒。

```text
src/commands/agent_tests_root_rust_policy.rs:146:6:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

3. `commands::agent_tests::coverage_only_result_never_fabricates_a_reviewer_or_a_confirmed_decision`

   原日志 `17.log`，exit101，1.39秒。

```text
src/commands/multi_agent/tests/review_gate.rs:79:68:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

4. `commands::agent_tests::e2e_pause_resume_and_reexecute_keep_the_budget`

   原日志 `25.log`，exit101，1.26秒。

```text
src/commands/agent_tests_browser.rs:54:80:
called `Result::unwrap()` on an `Err` value: "web_start_paid_history_requires_explicit_financial_contract"
```

5. `commands::agent_tests::executor_allocation_actual_three_sdk_preparation_preserves_original_reviewer_floor`

   原日志 `27.log`，exit101，1.64秒。

```text
src/commands/agent_tests_executor_allocation.rs:55:6:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

6. `commands::agent_tests::executor_allocation_full_mixed_unlimited_preparation_binds_actual_runtime_windows`

   原日志 `28.log`，exit101，1.5秒。

```text
src/commands/agent_tests_executor_allocation.rs:219:10:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

7. `commands::agent_tests::executor_mailbox_failure_releases_target_lane_and_reservation`

   原日志 `29.log`，exit101，1.46秒。

```text
src/commands/multi_agent/tests/scheduler.rs:200:5:
web_executor_original_exit_proof_missing
```

8. `commands::agent_tests::failed_gap_round_is_not_mistaken_for_an_assessed_revision`

   原日志 `30.log`，exit101，1.51秒。

```text
src/commands/multi_agent/tests/gap.rs:408:102:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

9. `commands::agent_tests::fresh_multi_authenticated_anonymous_control_actual_entry_preserves_account_metadata_and_fees`

   原日志 `31.log`，exit101，10.42秒。

```text
src/commands/agent_tests_multi_anonymous_control_original.rs:194:9:
actual SDK missing 独立ClientSide配置事实专家; outcome=executor_followup_failed:new_evidence:child_budget_reservation_exceeded_or_stale; executor_code=hard_request_budget; executor_reason=模型调用达到 12 次硬上限；覆盖 1 个族，未覆盖 0 个族，确认问题 0，模型请求 12 次，Token 1680; SDK=21; originalUsage=[("coordinator", 22, 0, "terminal"), ("spa_api_mapper", 0, 1, "terminal"), ("identity_session", 0, 1, "terminal"), ("external_surface", 0, 1, "terminal"), ("web_executor", 0, 12, "terminal")]
```

10. `commands::agent_tests::gap_refuses_missing_or_changed_review_seal_before_scheduling`

   原日志 `32.log`，exit101，1.43秒。

```text
src/commands/multi_agent/tests/gap.rs:77:102:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

11. `commands::agent_tests::gap_review_real_independent_receipt_closes_link_without_rewriting_old_root_or_fabricating_findings`

   原日志 `33.log`，exit101，12.28秒。

```text
src/commands/agent_tests_gap_review.rs:284:5:
assertion `left == right` failed
  left: 4
 right: 0
```

12. `commands::agent_tests::insufficient_finding_remains_reviewable_only_after_a_new_candidate_revision`

   原日志 `34.log`，exit101，1.39秒。

```text
src/commands/multi_agent/tests/review_gate.rs:109:68:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

13. `commands::agent_tests::investigator_allocation_admits_fixed_proposal_when_surplus_exceeds_floors`

   原日志 `35.log`，exit101，1.57秒。

```text
src/commands/agent_tests_investigator_allocation.rs:67:6:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

14. `commands::agent_tests::native_sdk_log_coordinator_changed_fact_real_gate_uses_next_original_call_and_zero_sdk_replay`

   原日志 `36.log`，exit101，2.02秒。

```text
src/commands/agent_tests_root_tick_sdk_changed.rs:69:31:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

15. `commands::agent_tests::policy_native_routes_without_runtime_adapter`

   原日志 `42.log`，exit101，8.75秒。

```text
src/commands/agent_tests_native_policy_original.rs:8:5:
executor_followup_failed:new_evidence:child_budget_reservation_exceeded_or_stale; executor_code=hard_request_budget; executor_reason=模型调用达到 12 次硬上限；覆盖 1 个族，未覆盖 0 个族，确认问题 0，模型请求 12 次，Token 1680
```

16. `commands::agent_tests::request_accounting_terminal_snapshot_preserves_aggregate_and_child_ownership`

   原日志 `44.log`，exit101，1.47秒。

```text
src/commands/agent_tests_request_accounting.rs:184:65:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

17. `commands::agent_tests::resumed_single_root_refuses_inherited_or_skipped_model_round`

   原日志 `45.log`，exit101，1.09秒。

```text
src/commands/agent_tests_browser.rs:194:92:
called `Result::unwrap()` on an `Err` value: "web_start_paid_history_requires_explicit_financial_contract"
```

18. `commands::agent_tests::reviewer_and_replay_preserve_manual_execution_stop_without_gap_autostart`

   原日志 `47.log`，exit101，1.35秒。

```text
src/commands/multi_agent/tests/review_gate.rs:56:72:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

19. `commands::agent_tests::reviewer_finish_failure_cannot_publish_an_acked_decision`

   原日志 `48.log`，exit101，1.33秒。

```text
src/commands/multi_agent/tests/findings.rs:188:68:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

20. `commands::agent_tests::reviewer_insufficiency_starts_one_read_only_gap_run_and_assessment`

   原日志 `49.log`，exit101，1.3秒。

```text
src/commands/multi_agent/tests/gap.rs:8:106:
called `Result::unwrap()` on an `Err` value: "web_executor_original_exit_proof_missing"
```

完整日志、原名单、逐项结果、替代用例结果、源码/binary SHA和failure-map.json位于上述证据目录。完整REM-A01—A14 goal active；生产恢复、多Root共享十维/USD消费者、真实平台和最终发布仍需各自验收。
