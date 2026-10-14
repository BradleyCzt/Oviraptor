CREATE TABLE IF NOT EXISTS skill_packages (
    id TEXT PRIMARY KEY,
    package_key TEXT NOT NULL,
    name TEXT NOT NULL,
    source TEXT NOT NULL,
    version TEXT NOT NULL,
    license TEXT NOT NULL,
    builtin INTEGER NOT NULL CHECK(builtin IN (0,1)),
    digest TEXT NOT NULL,
    file_count INTEGER NOT NULL CHECK(file_count BETWEEN 1 AND 512),
    total_bytes INTEGER NOT NULL CHECK(total_bytes BETWEEN 1 AND 16777216),
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS skill_package_entries (
    skill_id INTEGER PRIMARY KEY REFERENCES agent_skills(id) ON DELETE RESTRICT,
    package_id TEXT NOT NULL REFERENCES skill_packages(id) ON DELETE CASCADE,
    entry_path TEXT NOT NULL,
    UNIQUE(package_id,entry_path)
);
CREATE TABLE IF NOT EXISTS skill_package_files (
    package_id TEXT NOT NULL REFERENCES skill_packages(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    content BLOB NOT NULL,
    size_bytes INTEGER NOT NULL CHECK(size_bytes BETWEEN 0 AND 1048576),
    sha256 TEXT NOT NULL,
    text INTEGER NOT NULL CHECK(text IN (0,1)),
    kind TEXT NOT NULL CHECK(kind IN ('instructions','reference','script','resource')),
    PRIMARY KEY(package_id,path)
);
-- Package resources and entry text are one immutable reviewed unit. The old
-- text-only editor/importer cannot detach its instructions from provenance.
CREATE TRIGGER IF NOT EXISTS skill_package_no_text_overwrite
BEFORE UPDATE OF name,description,instructions ON agent_skills
WHEN EXISTS(SELECT 1 FROM skill_package_entries e WHERE e.skill_id=OLD.id)
BEGIN SELECT RAISE(ABORT,'packaged_skill_immutable'); END;
CREATE TRIGGER IF NOT EXISTS skill_package_no_entry_delete
BEFORE DELETE ON agent_skills
WHEN EXISTS(SELECT 1 FROM skill_package_entries e WHERE e.skill_id=OLD.id)
BEGIN SELECT RAISE(ABORT,'packaged_skill_immutable'); END;
