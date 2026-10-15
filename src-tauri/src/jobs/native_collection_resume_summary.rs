const COLLECTION_RULE_VERSION: &str = "fofa-normalization-v2";
const COVERAGE_RULE_VERSION: &str = "fofa-coverage-v2";

#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]
struct QueryCollectionSummary {
    company: String,
    category: String,
    query: String,
    status: String,
    complete: bool,
    coverage_complete: bool,
    coverage_reasons: Vec<String>,
    total_matches: Option<usize>,
    reported_totals: Vec<usize>,
    last_page: usize,
    fetched_rows: usize,
    valid_rows: usize,
    rejected_rows: usize,
    scope_filtered_rows: usize,
    limited: bool,
    requested_fields: Vec<String>,
    actual_field_sets: Vec<Vec<String>>,
    field_coverage_complete: bool,
    received_at_first: Option<String>,
    received_at_last: Option<String>,
    request_attempts: usize,
    reused_pages: usize,
    error_category: Option<String>,
}

#[derive(Clone, serde::Deserialize, serde::Serialize)]
struct CollectionSummary {
    schema_version: u32,
    execution_id: String,
    binding_sha256: String,
    status: String,
    complete: bool,
    coverage_complete: bool,
    coverage_status: String,
    coverage_reasons: Vec<String>,
    compatibility_reasons: Vec<String>,
    error_categories: Vec<String>,
    queries: usize,
    completed_queries: usize,
    failed_queries: usize,
    truncated_queries: usize,
    partial_queries: usize,
    incomplete_queries: usize,
    invalid_records: usize,
    fetched_rows: usize,
    valid_rows: usize,
    rejected_rows: usize,
    scope_filtered_rows: usize,
    source_candidates: usize,
    candidates: usize,
    merged_duplicates: usize,
    native: bool,
    reused_pages: usize,
    migrated_legacy_pages: usize,
    request_attempts: usize,
    requested_fields: Vec<String>,
    actual_field_sets: Vec<Vec<String>>,
    field_coverage_complete: bool,
    received_at_first: Option<String>,
    received_at_last: Option<String>,
    unknown_time_pages: usize,
    collection_rule_version: String,
    coverage_rule_version: String,
    receipt_rule_version: String,
    payload_hash_semantics: String,
    retention: native_collection_resume::RetentionReport,
    storage_retention: Option<serde_json::Value>,
    query_summaries: Vec<QueryCollectionSummary>,
}

impl CollectionSummary {
    fn pending(execution_id: &str, queries: usize, requested_fields: &[String]) -> Self {
        Self {
            schema_version: 2,
            execution_id: execution_id.into(),
            binding_sha256: String::new(),
            status: "running".into(),
            complete: false,
            coverage_complete: false,
            coverage_status: "unknown".into(),
            coverage_reasons: Vec::new(),
            compatibility_reasons: Vec::new(),
            error_categories: Vec::new(),
            queries,
            completed_queries: 0,
            failed_queries: 0,
            truncated_queries: 0,
            partial_queries: 0,
            incomplete_queries: queries,
            invalid_records: 0,
            fetched_rows: 0,
            valid_rows: 0,
            rejected_rows: 0,
            scope_filtered_rows: 0,
            source_candidates: 0,
            candidates: 0,
            merged_duplicates: 0,
            native: true,
            reused_pages: 0,
            migrated_legacy_pages: 0,
            request_attempts: 0,
            requested_fields: requested_fields.into(),
            actual_field_sets: Vec::new(),
            field_coverage_complete: false,
            received_at_first: None,
            received_at_last: None,
            unknown_time_pages: 0,
            collection_rule_version: COLLECTION_RULE_VERSION.into(),
            coverage_rule_version: COVERAGE_RULE_VERSION.into(),
            receipt_rule_version: native_collection_resume::RECEIPT_RULE_VERSION.into(),
            payload_hash_semantics: native_collection_resume::PAYLOAD_HASH_SEMANTICS.into(),
            retention: native_collection_resume::RetentionReport::default(),
            storage_retention: None,
            query_summaries: Vec::new(),
        }
    }

    fn refresh(
        &mut self,
        checkpoint: &native_collection_resume::CollectionCheckpoint,
    ) -> Result<(), String> {
        self.completed_queries = self.query_summaries.iter().filter(|q| q.complete).count();
        self.failed_queries = self
            .query_summaries
            .iter()
            .filter(|q| q.status == "error")
            .count();
        self.truncated_queries = self.query_summaries.iter().filter(|q| q.limited).count();
        self.partial_queries = self
            .query_summaries
            .iter()
            .filter(|q| !q.coverage_complete)
            .count();
        self.incomplete_queries = self.queries - self.completed_queries;
        self.complete = self.completed_queries == self.queries && self.failed_queries == 0;
        self.coverage_complete = self.complete
            && self.query_summaries.len() == self.queries
            && self.query_summaries.iter().all(|q| q.coverage_complete);
        self.field_coverage_complete = self.query_summaries.len() == self.queries
            && self
                .query_summaries
                .iter()
                .all(|q| q.field_coverage_complete);
        self.coverage_status = if self.coverage_complete {
            "proven"
        } else {
            "unproven"
        }
        .into();
        self.fetched_rows = self.query_summaries.iter().map(|q| q.fetched_rows).sum();
        self.valid_rows = self.query_summaries.iter().map(|q| q.valid_rows).sum();
        self.rejected_rows = self.query_summaries.iter().map(|q| q.rejected_rows).sum();
        self.invalid_records = self.rejected_rows;
        self.scope_filtered_rows = self
            .query_summaries
            .iter()
            .map(|q| q.scope_filtered_rows)
            .sum();
        self.request_attempts = self
            .query_summaries
            .iter()
            .map(|q| q.request_attempts)
            .sum();
        self.reused_pages = self.query_summaries.iter().map(|q| q.reused_pages).sum();
        for query in &self.query_summaries {
            for reason in &query.coverage_reasons {
                add_reason(&mut self.coverage_reasons, reason);
            }
            if let Some(category) = &query.error_category {
                add_reason(&mut self.error_categories, category);
            }
            for fields in &query.actual_field_sets {
                if !self.actual_field_sets.contains(fields) {
                    self.actual_field_sets.push(fields.clone());
                }
            }
            observe_received(
                &mut self.received_at_first,
                &mut self.received_at_last,
                query.received_at_first.as_deref(),
            );
            observe_received(
                &mut self.received_at_first,
                &mut self.received_at_last,
                query.received_at_last.as_deref(),
            );
        }
        self.retention = checkpoint.retention_report();
        self.migrated_legacy_pages = self.retention.legacy_unknown_pages;
        self.unknown_time_pages = self.retention.legacy_unknown_pages;
        if self.unknown_time_pages > 0 {
            add_reason(&mut self.compatibility_reasons, "legacy_time_unknown");
        }
        if self.retention.expired_pages > 0 {
            add_reason(&mut self.compatibility_reasons, "receipt_expired");
        }
        self.storage_retention = checkpoint.storage_report()?;
        Ok(())
    }

    fn state(&self) -> Result<serde_json::Value, String> {
        serde_json::to_value(self).map_err(|error| error.to_string())
    }

    fn fail(&mut self, category: &str) {
        self.status = "incomplete".into();
        self.complete = false;
        self.coverage_complete = false;
        self.coverage_status = "unproven".into();
        add_reason(&mut self.error_categories, category);
        add_reason(&mut self.coverage_reasons, category);
    }
}

fn add_reason(reasons: &mut Vec<String>, code: &str) {
    if !reasons.iter().any(|reason| reason == code) {
        reasons.push(code.into());
    }
}

fn observe_received(first: &mut Option<String>, last: &mut Option<String>, value: Option<&str>) {
    let Some(time) = value.and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
    else {
        return;
    };
    let value = time
        .with_timezone(&chrono::Utc)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    if first.as_ref().is_none_or(|old| value < *old) {
        *first = Some(value.clone());
    }
    if last.as_ref().is_none_or(|old| value > *old) {
        *last = Some(value);
    }
}
