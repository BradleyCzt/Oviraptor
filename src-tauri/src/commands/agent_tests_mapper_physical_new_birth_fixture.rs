struct MapperPhysicalNewBirthScope {
    db_path: PathBuf,
    scan: String,
    attempt: i64,
    target: String,
    plan_hash: Option<String>,
    fired: usize,
}
thread_local! {
    static MAPPER_PHYSICAL_NEW_BIRTH: std::cell::RefCell<Option<MapperPhysicalNewBirthScope>> = const { std::cell::RefCell::new(None) };
}
struct MapperPhysicalNewBirthGuard;
impl MapperPhysicalNewBirthGuard {
    fn install(h: &AgentHarness) -> Self {
        MAPPER_PHYSICAL_NEW_BIRTH.with(|slot| {
            assert!(slot.borrow().is_none());
            *slot.borrow_mut() = Some(MapperPhysicalNewBirthScope {
                db_path: h.db_path.clone(),
                scan: h.context.scan_id.clone(),
                attempt: h.context.attempt_number,
                target: h.context.target_url.clone(),
                plan_hash: None,
                fired: 0,
            });
        });
        Self
    }
}
impl Drop for MapperPhysicalNewBirthGuard {
    fn drop(&mut self) {
        MAPPER_PHYSICAL_NEW_BIRTH.with(|slot| {
            let scope = slot.borrow_mut().take().unwrap();
            if !std::thread::panicking() {
                assert!(scope.fired > 0);
            }
        });
    }
}
pub(super) fn native_mapper_physical_new_birth_for_test(
    db: &rusqlite::Connection,
    db_path: &Path,
    scan: &str,
    attempt: i64,
    target: &str,
    plan: &AgentExecutionPlan,
    declaration: crate::agent_runtime::web_mode::root::NewRootModeDeclaration,
) -> Result<crate::agent_runtime::web_mode::root::NewRootModeDeclaration, String> {
    MAPPER_PHYSICAL_NEW_BIRTH.with(|slot| {
        let mut scoped = slot.borrow_mut();
        let Some(scope) = scoped.as_mut() else {
            return Ok(declaration);
        };
        if scope.db_path != db_path
            || scope.scan != scan
            || scope.attempt != attempt
            || scope.target != target
        {
            return Ok(declaration);
        }
        let hash = plan.hash();
        if scope
            .plan_hash
            .as_ref()
            .is_some_and(|original| original != &hash)
            || db
                .query_row(
                    "SELECT count(*) FROM agent_runs WHERE scan_id=?1",
                    [scan],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(|e| e.to_string())?
                != 0
        {
            return Err("mapper_physical_new_birth_scope_conflict".into());
        }
        scope.plan_hash = Some(hash);
        scope.fired += 1;
        declaration.with_mapper_v15_bootstrap_for_test()
    })
}
