-- Creation-only v8 HTTP cost evidence. No grant, replay, refund or backfill.
CREATE TABLE IF NOT EXISTS agent_http_budget_receipts (
 receipt_id TEXT PRIMARY KEY NOT NULL CHECK(length(receipt_id)=64),
 root_run_id TEXT NOT NULL CHECK(length(root_run_id)>0),
 source_id TEXT NOT NULL CHECK(length(source_id)>0),
 scan_id TEXT NOT NULL CHECK(length(scan_id)>0),
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 target_url TEXT NOT NULL CHECK(length(target_url)>0),
 run_id TEXT NOT NULL CHECK(length(run_id)>0),
 response_status INTEGER NOT NULL CHECK(response_status BETWEEN 100 AND 599),
 fact_json TEXT NOT NULL CHECK(json_valid(fact_json)),
 fact_hash TEXT NOT NULL CHECK(length(fact_hash)=64 AND fact_hash=receipt_id),
 signature BLOB NOT NULL CHECK(typeof(signature)='blob' AND length(signature)=32),
 CHECK(COALESCE(json_extract(fact_json,'$.schemaVersion')=1,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.purpose')='original_received_http_target_cost',0)),
 CHECK(COALESCE(json_extract(fact_json,'$.rootRunId')=root_run_id,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.sourceId')=source_id,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.scanId')=scan_id,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.attemptNumber')=attempt_number,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.targetUrl')=target_url,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.claim.runId')=run_id,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.status')=response_status,0)),
 CHECK(COALESCE(json_extract(fact_json,'$.targetRequests')=1,0)),
 UNIQUE(root_run_id,source_id)
) WITHOUT ROWID;
CREATE TRIGGER IF NOT EXISTS http_budget_receipt_no_update
 BEFORE UPDATE ON agent_http_budget_receipts
 BEGIN SELECT RAISE(ABORT,'http_budget_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS http_budget_receipt_no_delete
 BEFORE DELETE ON agent_http_budget_receipts
 BEGIN SELECT RAISE(ABORT,'http_budget_receipt_immutable'); END;
CREATE TRIGGER IF NOT EXISTS http_budget_receipt_no_replace
 BEFORE INSERT ON agent_http_budget_receipts
 WHEN EXISTS(SELECT 1 FROM agent_http_budget_receipts old
  WHERE old.receipt_id=NEW.receipt_id
   OR (old.root_run_id=NEW.root_run_id AND old.source_id=NEW.source_id))
 BEGIN SELECT RAISE(ABORT,'http_budget_receipt_immutable'); END;
