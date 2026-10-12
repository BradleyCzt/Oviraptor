> 主控最新只读核对：两处Result已由外部工作树改为unwrap；主控没有编辑Exposure测试。全库严格复验正在运行，成功终态取得前不能认为gate已通过。

# Qoder Exposure 严格检查依赖

主控2026-10-09检查 `cargo clippy --lib --tests -- -D warnings` 失败，日志 `/tmp/oviraptor-master-capacity-concurrency-20261008/authorization-output-connected-clippy.log`。当前 `src-tauri/src/commands/exposure/tests_g4_scale.rs` 第810行安装 `progress_handler` 与815行移除处理器返回 Result，均未处理。请在专属Exposure测试内明确验安装/移除成功（例如测试中 unwrap/expect），保持原性能/进度断言，不通过 ignore/allow/丢弃结果换绿。主控不覆盖这些业务测试。修复后通知主控统一全库严格复验；当前生产lib通过与否不能替代lib+tests gate。

Authorization 主控功能与Exposure实现分工不变；不改主控预算/派发/Root协议，不触真实数据。
