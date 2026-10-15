import type { SentinelFuseEntry, SentinelTarget } from "../types";

// Fuse copy and classification belong to the stop/disposition feature. These
// helpers only read saved evidence; they never decide whether a scan can resume.
export function fuseReasonParts(item: SentinelFuseEntry, target?: SentinelTarget) {
  const raw = target?.routingReason || item.reason || "";
  return String(raw)
    .split("；")
    .map((text) => text.trim())
    .filter(Boolean)
    .map((text) => {
      let tone = "general",
        label = "扫描信号";
      if (/HTTP|入口可访问|入口受限|入口响应/.test(text)) {
        tone = "access";
        label = "入口";
      } else if (/SourceMap/i.test(text)) {
        tone = "sourcemap";
        label = "SourceMap";
      } else if (/业务脚本|应用分包|JS/i.test(text)) {
        tone = "javascript";
        label = "JS";
      } else if (/识别到/.test(text)) {
        tone = "framework";
        label = "框架";
      } else if (/API/.test(text)) {
        tone = "api";
        label = "API";
      } else if (/路由/.test(text)) {
        tone = "route";
        label = "路由";
      } else if (/敏感|鉴权|管理|上传|业务入口/.test(text)) {
        tone = "sensitive";
        label = "敏感业务";
      } else if (/熔断|模型调用|Token|无进展|计划\/待办/.test(text)) {
        tone = "fuse";
        label = "熔断";
      }
      return { text, tone, label };
    });
}

export function fuseReasonCategory(item: SentinelFuseEntry, target?: SentinelTarget) {
  const reason = `${item.reason} ${target?.routingReason || ""}`.toLowerCase();
  if (/token|预算|budget|无进展|no.?progress|模型调用|计划\/待办/.test(reason))
    return "budget";
  if (/401|403|鉴权|登录|认证|unauthor|forbidden|access denied/.test(reason))
    return "access";
  if (/waf|拦截|验证码|captcha|限流|rate.?limit|封禁|anti.?bot/.test(reason))
    return "blocked";
  if (/timeout|超时|连接|network|dns|tls|证书|异常|error|failed/.test(reason))
    return "failure";
  return "low_value";
}

export function fuseCategoryLabel(category: string) {
  return ({
    budget: "成本 / 无进展",
    access: "缺少访问条件",
    blocked: "遭到拦截",
    failure: "网络 / 执行异常",
    low_value: "价值不足",
  } as Record<string, string>)[category] || category;
}

export function fuseRecommendedAction(item: SentinelFuseEntry, target?: SentinelTarget) {
  return ({
    budget: "先看已保存情报；有明确接口或参数再恢复，避免继续空烧 Token。",
    access: "补充 Cookie、Token 或登录态后恢复重试。",
    blocked: "保持停止；确认访问策略或降低频率后再恢复。",
    failure: "确认网络、DNS 或证书状态，修复环境后直接重试。",
    low_value: "快速人工复核现有 JS/API 证据；无新增价值即可归档。",
  } as Record<string, string>)[fuseReasonCategory(item, target)];
}
