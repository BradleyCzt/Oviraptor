import { invoke } from "@tauri-apps/api/core";
import type { SecurityRulePack, SecurityRulePackInput } from "./rulePacksTypes";

/** Scanning owns executable rule repositories and explicit synchronization. */
export const scanningRulePacksApi = {
  listSecurityRulePacks: () => invoke<SecurityRulePack[]>("list_security_rule_packs"),
  saveSecurityRulePack: (input: SecurityRulePackInput) => invoke<number>("save_security_rule_pack", { input }),
  deleteSecurityRulePack: (packId: number) => invoke<void>("delete_security_rule_pack", { packId }),
  syncSecurityRulePack: (packId: number) => invoke<SecurityRulePack>("sync_security_rule_pack", { packId }),
};
