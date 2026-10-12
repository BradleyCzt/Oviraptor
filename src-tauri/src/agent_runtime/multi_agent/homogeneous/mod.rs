//! Phase 1「同质 Worker + 强主管」控制面。
//!
//! DESIGN §2.1 的模块划分：契约解析（decision）、候选（allowlist）、状态迁移
//! （apply + actions）、机器可检（machine）、确定性降级（decompose）、引导闸门
//! （directive_gate）、落库（store）。所有副作用只允许经 `apply::apply_decision`
//! 进入；legacy 固定角色管线不读这里的任何状态。

// Phase 1.1 接线状态（2026-10-11）：Root 付费 tick 的返回路径调用
// `apply::run_coordinator_turn`；`assign` 经 `delivery` 建立 child run / attempt /
// mailbox 载体；`worker::drain_for_lease` 在同一个 tick 尾巴上消费
// `homogeneous_slice_dispatch`（现网收方 API 领取 → mark_started → 只读执行 →
// `record_worker_report`）；到期回收既在每轮决策前触发，也挂在
// `supervisor::sweep` 的心跳上（`pool::reclaim_expired_for_root`）；prepare 阶段
// 对显式 homogeneous_workers 的扫描计划直接让路，不再进入 legacy bootstrap dispatch。
// 本模块仍保留少量「有 API 无调用方」条目（只读复核护栏），与
// multi_agent/assignment.rs 同一约定。
#![allow(dead_code)]

pub(crate) mod allowlist;
pub(crate) mod apply;
pub(crate) mod budget_bridge;
pub(crate) mod decision;
pub(crate) mod decompose;
pub(crate) mod delivery;
pub(crate) mod directive_gate;
pub(crate) mod gate;
pub(crate) mod intent;
pub(crate) mod journal;
pub(crate) mod machine;
pub(crate) mod material_policy;
pub(crate) mod model_worker;
pub(crate) mod execution_policy;
pub(crate) mod parser_execution;
pub(crate) mod execution_worker;
pub(crate) mod pool;
pub(crate) mod prompt;
pub(crate) mod reason;
pub(crate) mod store;
pub(crate) mod transitions;
pub(crate) mod types;
pub(crate) mod worker;

#[cfg(test)]
mod apply_tests;
#[cfg(test)]
mod budget_bridge_tests;
#[cfg(test)]
mod decision_tests;
#[cfg(test)]
mod decompose_tests;
#[cfg(test)]
mod directive_gate_tests;
#[cfg(test)]
mod golden_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod intent_tests;
#[cfg(test)]
mod machine_tests;
#[cfg(test)]
mod material_policy_tests;
#[cfg(test)]
mod test_fixture;

pub mod model_guidance;

pub(crate) mod execution_carrier;
