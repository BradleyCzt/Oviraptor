use super::{import, model::Package, store};
use rusqlite::{params, Connection};
use std::{fs, path::PathBuf};

struct Folder(PathBuf);
impl Folder {
    fn new() -> Self {
        let path=std::env::temp_dir().join(format!("oviraptor-skill-package-{}",uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn put(&self,path:&str,bytes:impl AsRef<[u8]>) {
        let path=self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path,bytes).unwrap();
    }
    fn skill(&self,path:&str,name:&str) {
        self.put(path,format!("---\nname: {name}\ndescription: Synthetic package fixture\n---\nRead the selected project only.\n"));
    }
    fn package(&self)->Package {import::local(&self.0).unwrap()}
}
impl Drop for Folder {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}

fn database()->(Folder,Connection) {
    let folder=Folder::new();
    let path=crate::db::initialize(&folder.0).unwrap();
    let db=crate::db::open(&path).unwrap();
    (folder,db)
}
fn count(db:&Connection,table:&str)->i64 {
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"),[],|r|r.get(0)).unwrap()
}
fn packaged_id(db:&Connection,name:&str)->i64 {
    db.query_row("SELECT id FROM agent_skills WHERE name=?1",[name],|r|r.get(0)).unwrap()
}

#[test]
fn bundle_preserves_exact_upstream_identity_and_all_36_skills_443_files() {
    let package=import::bundled().unwrap();
    assert_eq!(package.id,"every-compound-engineering");
    assert_eq!(package.source,"https://github.com/EveryInc/compound-engineering-plugin");
    assert_eq!(package.version,"50532b00de523980010d97fe950965b35382a3b7");
    assert_eq!(package.license,"MIT");
    assert_eq!(package.skills.len(),36);
    assert_eq!(package.files.len(),443);
    let license=package.files.iter().find(|f|f.path=="LICENSE").unwrap();
    assert!(std::str::from_utf8(&license.bytes).unwrap().contains("MIT License"));
    for skill in &package.skills {
        let file=package.files.iter().find(|f|f.path==skill.entry_path).unwrap();
        assert_eq!(skill.instructions.as_bytes(),file.bytes);
    }
    let (_folder,mut db)=database();
    assert!(!store::installed(&db).unwrap(),"startup only creates tables");
    let saved=store::install(&mut db,package).unwrap();
    assert_eq!(saved.skill_count,36);
    assert_eq!(saved.file_count,443);
    assert_eq!(count(&db,"skill_package_files"),443);
    let preserved:Vec<u8>=db.query_row("SELECT content FROM skill_package_files WHERE package_id=?1 AND path='LICENSE'",[&saved.id],|r|r.get(0)).unwrap();
    assert_eq!(preserved,import::bundled().unwrap().files.into_iter().find(|f|f.path=="LICENSE").unwrap().bytes);
    assert!(saved.missing_capabilities.iter().any(|message|message.contains("inert")));
}

#[test]
fn sibling_skill_directory_import_preserves_root_legal_notices_and_resources() {
    let folder=Folder::new();
    folder.skill("alpha/SKILL.md","alpha");
    folder.skill("beta/SKILL.md","beta");
    folder.put("alpha/references/guide.txt","project guidance");
    folder.put("LICENSE","Synthetic permissive license");
    folder.put("NOTICE","Synthetic provenance notice");
    let package=folder.package();
    assert_eq!(package.skills.len(),2);
    assert_eq!(package.files.len(),5);
    assert!(package.files.iter().any(|file|file.path=="LICENSE"&&file.bytes==b"Synthetic permissive license"));
    assert!(package.files.iter().any(|file|file.path=="NOTICE"&&file.bytes==b"Synthetic provenance notice"));
}

#[test]
fn local_preview_digest_is_stable_and_changes_with_resource_content() {
    let folder=Folder::new();folder.skill("SKILL.md","digest-test");
    folder.put("references/guide.txt","first");
    let first=folder.package();
    assert_eq!(first.digest,folder.package().digest);
    folder.put("references/guide.txt","second");
    let second=folder.package();
    assert_ne!(first.digest,second.digest);
    assert_ne!(first.id,second.id);
    let preview=import::preview(&second);
    assert_eq!(preview.digest,second.digest);
    assert_eq!(preview.file_count,2);
    assert!(preview.warnings.iter().any(|warning|warning.contains("does not grant")));
}

#[test]
fn unsafe_relative_paths_are_rejected_without_normalizing_identity() {
    for path in ["","/SKILL.md","./SKILL.md","../SKILL.md","a/../SKILL.md","a//b","a\\b","C:foo","a/./b","a/secret.","a/secret ","a/\nsecret"] {
        assert!(import::checked_path(path).is_err(),"accepted {path:?}");
    }
    for path in ["SKILL.md","skills/a/references/example.md","中文/参考.txt"] {assert!(import::checked_path(path).is_ok());}
}

#[cfg(unix)]
#[test]
fn literal_backslash_filename_is_rejected_instead_of_importing_a_different_path() {
    let folder=Folder::new();folder.skill("SKILL.md","backslash-test");
    folder.put("references\\alternate.txt","a distinct Unix file");
    assert_eq!(import::local(&folder.0).err().unwrap(),"skill_package_unsafe_path");
}

#[cfg(unix)]
#[test]
fn import_rejects_symlink_roots_files_and_directories() {
    use std::os::unix::fs::symlink;
    let outside=Folder::new();outside.skill("SKILL.md","outside");
    let folder=Folder::new();
    symlink(&outside.0,folder.0.join("root-link")).unwrap();
    assert_eq!(import::local(&folder.0.join("root-link")).err().unwrap(),"skill_package_symlink_forbidden");
    folder.skill("SKILL.md","inside");
    symlink(outside.0.join("SKILL.md"),folder.0.join("linked.md")).unwrap();
    assert_eq!(import::local(&folder.0).err().unwrap(),"skill_package_symlink_forbidden");
    fs::remove_file(folder.0.join("linked.md")).unwrap();
    fs::remove_file(folder.0.join("root-link")).unwrap();
    symlink(&outside.0,folder.0.join("references")).unwrap();
    assert_eq!(import::local(&folder.0).err().unwrap(),"skill_package_symlink_forbidden");
}

#[test]
fn import_rejects_duplicate_case_insensitive_names_and_paths() {
    let folder=Folder::new();folder.skill("skills/a/SKILL.md","duplicate");folder.skill("skills/b/SKILL.md","DUPLICATE");
    assert_eq!(import::local(&folder.0).err().unwrap(),"skill_package_duplicate_skill_name");
    let folder=Folder::new();folder.skill("SKILL.md","unique");folder.put("skill.md","same path on case-insensitive systems");
    // Case-insensitive filesystems already identify these as the same file.
    if fs::read_dir(&folder.0).unwrap().count()==2 {
        assert_eq!(import::local(&folder.0).err().unwrap(),"skill_package_duplicate_path");
    }
}

#[test]
fn import_enforces_file_and_instruction_limits() {
    let folder=Folder::new();folder.skill("SKILL.md","limits");
    let file=fs::File::create(folder.0.join("large.txt")).unwrap();file.set_len((import::MAX_FILE_BYTES+1) as u64).unwrap();
    assert_eq!(import::local(&folder.0).err().unwrap(),"skill_package_file_too_large");
    fs::remove_file(folder.0.join("large.txt")).unwrap();
    folder.put("SKILL.md","x".repeat(30_001));
    assert!(import::local(&folder.0).err().unwrap().starts_with("skill_package_invalid_instructions:"));
}

#[test]
fn custom_name_collision_is_atomic_and_preserves_existing_skill() {
    let folder=Folder::new();folder.skill("skills/a/SKILL.md","new-unique");folder.skill("skills/b/SKILL.md","existing-custom");
    let (_db_folder,mut db)=database();
    db.execute("INSERT INTO agent_skills(name,instructions) VALUES('EXISTING-CUSTOM','original local instructions')",[]).unwrap();
    let before=count(&db,"agent_skills");
    assert_eq!(store::install(&mut db,folder.package()).err().unwrap(),"skill_package_name_conflict:existing-custom");
    assert_eq!(count(&db,"agent_skills"),before);
    assert_eq!(count(&db,"skill_packages"),0);
    assert_eq!(count(&db,"skill_package_files"),0);
    assert_eq!(count(&db,"skill_package_entries"),0);
    let instructions:String=db.query_row("SELECT instructions FROM agent_skills WHERE name='EXISTING-CUSTOM'",[],|r|r.get(0)).unwrap();
    assert_eq!(instructions,"original local instructions");
}

#[test]
fn lazy_bundle_registration_survives_custom_name_and_namespace_collisions() {
    let package=import::bundled().unwrap();
    let original=package.skills[0].name.clone();
    let entry=package.skills[0].entry_path.clone();
    let instructions=package.skills[0].instructions.clone();
    let (_folder,mut db)=database();
    for name in [original.clone(),format!("{original} (Every Compound)")] {
        db.execute("INSERT INTO agent_skills(name,instructions) VALUES(?1,'owned custom instructions')",[name]).unwrap();
    }
    if !store::installed(&db).unwrap(){store::install(&mut db,package).unwrap();}
    let packages=store::list(&db).unwrap();
    assert_eq!(packages.len(),1);
    assert_eq!(packages[0].skill_count,36);
    let installed=packages[0].skills.iter().find(|skill|skill.entry_path==entry).unwrap();
    assert_eq!(installed.name,format!("{original} (Every Compound 2)"));
    let retained:String=db.query_row("SELECT instructions FROM agent_skills WHERE name=?1",[&original],|r|r.get(0)).unwrap();
    assert_eq!(retained,"owned custom instructions");
    let upstream:String=db.query_row("SELECT instructions FROM agent_skills WHERE id=?1",[installed.id],|r|r.get(0)).unwrap();
    assert_eq!(upstream,instructions);
    assert_eq!(store::install(&mut db,import::bundled().unwrap()).unwrap().skill_count,36);
}

#[test]
fn package_install_is_idempotent_and_legacy_editor_cannot_detach_provenance() {
    let folder=Folder::new();folder.skill("SKILL.md","immutable");
    let (_db_folder,mut db)=database();
    let first=store::install(&mut db,folder.package()).unwrap();
    let second=store::install(&mut db,folder.package()).unwrap();
    assert_eq!(first.id,second.id);assert_eq!(first.skills[0].id,second.skills[0].id);
    assert_eq!(count(&db,"skill_packages"),1);
    let id=first.skills[0].id;
    assert!(db.execute("UPDATE agent_skills SET instructions='replaced' WHERE id=?1",[id]).is_err());
    assert!(db.execute("DELETE FROM agent_skills WHERE id=?1",[id]).is_err());
    store::set_enabled(&mut db,id,false).unwrap();
    assert!(!store::info(&db,&first.id).unwrap().skills[0].enabled);
    store::set_enabled(&mut db,id,true).unwrap();
    assert!(store::info(&db,&first.id).unwrap().skills[0].enabled);
    assert_eq!(store::set_enabled(&mut db,i64::MAX,true).err().unwrap(),"packaged_skill_unavailable");
}

#[test]
fn skill_resources_are_scoped_and_scripts_remain_inert() {
    let folder=Folder::new();folder.skill("skills/a/SKILL.md","scope-a");folder.skill("skills/b/SKILL.md","scope-b");
    folder.put("skills/a/references/guide.txt","a reference");
    folder.put("skills/a/scripts/inert.sh","#!/bin/sh\nexit 99\n");
    folder.put("skills/b/references/private.txt","b reference");folder.put("LICENSE","Synthetic license");
    folder.put("skills/a/.env","synthetic fixture omitted from reads");
    let (_db_folder,mut db)=database();store::install(&mut db,folder.package()).unwrap();
    let id=packaged_id(&db,"scope-a");
    let resources=store::list_resources(&db,id).unwrap();
    let resources=resources["resources"].as_array().unwrap();
    assert!(resources.iter().any(|row|row["path"]=="LICENSE"));
    assert!(resources.iter().any(|row|row["path"]=="skills/a/scripts/inert.sh"&&row["kind"]=="script"));
    assert!(!resources.iter().any(|row|row["path"].as_str().unwrap().contains("/b/")||row["path"].as_str().unwrap().ends_with(".env")));
    assert_eq!(store::read_resource(&db,id,"skills/b/SKILL.md",0,4096).err().unwrap(),"skill_package_resource_denied");
    assert_eq!(store::read_resource(&db,id,"skills/a/.env",0,4096).err().unwrap(),"skill_package_resource_denied");
    assert_eq!(store::read_resource(&db,id,"skills/a/../b/SKILL.md",0,4096).err().unwrap(),"skill_package_unsafe_path");
    assert_eq!(store::read_resource(&db,id,"LICENSE",0,4096).unwrap()["content"],"Synthetic license");
}

#[test]
fn resource_pages_preserve_utf8_and_validate_offsets_and_byte_limits() {
    let folder=Folder::new();folder.skill("SKILL.md","paged");folder.put("references/unicode.txt","a中😀z");
    let (_db_folder,mut db)=database();store::install(&mut db,folder.package()).unwrap();let id=packaged_id(&db,"paged");
    let first=store::read_resource(&db,id,"references/unicode.txt",0,5).unwrap();
    assert_eq!(first["content"],"a中");assert_eq!(first["nextOffset"],4);assert_eq!(first["totalBytes"],9);
    let second=store::read_resource(&db,id,"references/unicode.txt",4,5).unwrap();
    assert_eq!(second["content"],"😀z");assert_eq!(second["hasMore"],false);assert!(second["nextOffset"].is_null());
    assert_eq!(store::read_resource(&db,id,"references/unicode.txt",2,5).err().unwrap(),"skill_package_offset_invalid");
    assert_eq!(store::read_resource(&db,id,"references/unicode.txt",10,5).err().unwrap(),"skill_package_offset_invalid");
    assert_eq!(store::read_resource(&db,id,"references/unicode.txt",4,1).err().unwrap(),"skill_package_page_too_small");
    for limit in [0,8193] {assert_eq!(store::read_resource(&db,id,"references/unicode.txt",0,limit).err().unwrap(),"skill_package_page_limit");}
}

#[test]
fn resources_reject_integrity_changes_and_do_not_return_binary_content() {
    let folder=Folder::new();folder.skill("SKILL.md","integrity");folder.put("references/data.bin",[0,159,146,150]);folder.put("references/readme.txt","untampered");
    let (_db_folder,mut db)=database();let saved=store::install(&mut db,folder.package()).unwrap();let id=saved.skills[0].id;
    let binary=store::read_resource(&db,id,"references/data.bin",0,4096).unwrap();
    assert!(binary["content"].is_null());assert_eq!(binary["encoding"],"binary");
    db.execute("UPDATE skill_package_files SET content=?2 WHERE package_id=?1 AND path='references/readme.txt'",params![saved.id,b"changed".as_slice()]).unwrap();
    assert_eq!(store::read_resource(&db,id,"references/readme.txt",0,4096).err().unwrap(),"skill_package_resource_integrity_failed");
}

#[test]
fn credential_detection_scans_entire_resource_before_any_page() {
    let folder=Folder::new();folder.skill("SKILL.md","credential-fixture");
    // A clearly synthetic assignment exercises the boundary without real credentials.
    folder.put("references/example.txt",format!("{}\napi_key = \"synthetic-example-only\"\n","safe guidance ".repeat(100)));
    let (_db_folder,mut db)=database();store::install(&mut db,folder.package()).unwrap();let id=packaged_id(&db,"credential-fixture");
    assert_eq!(store::read_resource(&db,id,"references/example.txt",0,16).err().unwrap(),"skill_package_credential_content_denied");
}

#[test]
fn consent_fingerprint_binds_provenance_entry_and_actual_accessible_resources() {
    let folder=Folder::new();folder.skill("skills/a/SKILL.md","fingerprint-a");folder.skill("skills/b/SKILL.md","fingerprint-b");
    folder.put("skills/a/references/guide.txt","first guidance");folder.put("skills/b/references/guide.txt","other guidance");folder.put("LICENSE","Synthetic license");
    let (_db_folder,mut db)=database();let saved=store::install(&mut db,folder.package()).unwrap();let id=packaged_id(&db,"fingerprint-a");
    let first=store::package_fingerprint(&db,id).unwrap().unwrap();
    assert_eq!(store::package_fingerprint(&db,i64::MAX).unwrap(),None);
    assert_eq!(store::package_fingerprint(&db,id).unwrap().unwrap(),first);
    db.execute("UPDATE skill_packages SET source='Synthetic amended source' WHERE id=?1",[&saved.id]).unwrap();
    let provenance=store::package_fingerprint(&db,id).unwrap().unwrap();assert_ne!(provenance,first);
    let bytes=b"reviewed updated guidance";
    db.execute("UPDATE skill_package_files SET content=?2,size_bytes=?3,sha256=?4 WHERE package_id=?1 AND path='skills/a/references/guide.txt'",params![saved.id,bytes.as_slice(),bytes.len() as i64,import::hash(bytes)]).unwrap();
    let resource=store::package_fingerprint(&db,id).unwrap().unwrap();assert_ne!(resource,provenance);
    let bytes=b"changed sibling guidance";
    db.execute("UPDATE skill_package_files SET content=?2,size_bytes=?3,sha256=?4 WHERE package_id=?1 AND path='skills/b/references/guide.txt'",params![saved.id,bytes.as_slice(),bytes.len() as i64,import::hash(bytes)]).unwrap();
    assert_eq!(store::package_fingerprint(&db,id).unwrap().unwrap(),resource,"unselected sibling entry is outside this grant");
    let bytes=b"Synthetic revised license";
    db.execute("UPDATE skill_package_files SET content=?2,size_bytes=?3,sha256=?4 WHERE package_id=?1 AND path='LICENSE'",params![saved.id,bytes.as_slice(),bytes.len() as i64,import::hash(bytes)]).unwrap();
    assert_ne!(store::package_fingerprint(&db,id).unwrap().unwrap(),resource,"shared legal provenance belongs to every grant");
    db.execute("UPDATE skill_package_files SET content=?2 WHERE package_id=?1 AND path='skills/a/references/guide.txt'",params![saved.id,b"corrupted".as_slice()]).unwrap();
    assert_eq!(store::package_fingerprint(&db,id).err().unwrap(),"skill_package_resource_integrity_failed");
}
