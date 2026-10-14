use super::{import,model::*};
use rusqlite::{params,Connection,OptionalExtension,TransactionBehavior};
use serde_json::{json,Value};

fn err(e:rusqlite::Error)->String{format!("skill_package_storage:{e}")}
fn stamp()->String{chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis,true)}

pub(super) fn info(db:&Connection,id:&str)->Result<PackageInfo,String>{
    let mut package:PackageInfo=db.query_row(
        "SELECT id,name,source,version,license,builtin,digest,file_count,total_bytes,created_at FROM skill_packages WHERE id=?1",[id],|r|Ok(PackageInfo{
            id:r.get(0)?,name:r.get(1)?,source:r.get(2)?,version:r.get(3)?,license:r.get(4)?,builtin:r.get(5)?,digest:r.get(6)?,file_count:r.get::<_,i64>(7)? as usize,total_bytes:r.get::<_,i64>(8)? as usize,created_at:r.get(9)?,skill_count:0,skills:Vec::new(),missing_capabilities:capability_notices(),
        }),
    ).optional().map_err(err)?.ok_or("skill_package_unavailable")?;
    let mut statement=db.prepare("SELECT s.id,s.name,s.description,e.entry_path,s.enabled FROM skill_package_entries e JOIN agent_skills s ON s.id=e.skill_id WHERE e.package_id=?1 ORDER BY s.name,s.id").map_err(err)?;
    package.skills=statement.query_map([id],|r|Ok(SkillSummary{id:r.get(0)?,name:r.get(1)?,description:r.get(2)?,entry_path:r.get(3)?,enabled:r.get(4)?})).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    package.skill_count=package.skills.len();Ok(package)
}

pub(super) fn list(db:&Connection)->Result<Vec<PackageInfo>,String>{
    let mut statement=db.prepare("SELECT id FROM skill_packages ORDER BY builtin DESC,name,id").map_err(err)?;
    let ids=statement.query_map([],|r|r.get::<_,String>(0)).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    ids.iter().map(|id|info(db,id)).collect()
}

pub(super) fn installed(db:&Connection)->Result<bool,String>{
    db.query_row("SELECT EXISTS(SELECT 1 FROM skill_packages WHERE id='every-compound-engineering')",[],|r|r.get(0)).map_err(err)
}

pub(super) fn install(db:&mut Connection,mut package:Package)->Result<PackageInfo,String>{
    let tx=db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(err)?;
    let previous:Option<String>=tx.query_row("SELECT digest FROM skill_packages WHERE id=?1",[&package.id],|r|r.get(0)).optional().map_err(err)?;
    if let Some(previous)=previous{
        if previous!=package.digest{return Err("skill_package_version_conflict".into());}
        return info(&tx,&package.id);
    }
    // Custom imports remain all-or-nothing. The lazy built-in registration must
    // also remain usable when a user already owns one of the upstream names.
    // Namespace only its display name; retain the original source instructions,
    // entry path and package digest as the reviewed upstream unit.
    for index in 0..package.skills.len(){
        let original=package.skills[index].name.clone();
        let mut candidate=original.clone();
        let mut ordinal=0usize;
        loop {
            let exists:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM agent_skills WHERE name=?1 COLLATE NOCASE)",[&candidate],|r|r.get(0)).map_err(err)?;
            let reserved=package.skills.iter().enumerate().any(|(other_index,other)|other_index!=index&&other.name.eq_ignore_ascii_case(&candidate));
            if !exists && !reserved {break;}
            if !package.builtin{return Err(format!("skill_package_name_conflict:{}",original));}
            ordinal+=1;
            let suffix=if ordinal==1{" (Every Compound)".to_string()}else{format!(" (Every Compound {ordinal})")};
            let prefix:String=original.chars().take(80-suffix.chars().count()).collect();
            candidate=format!("{prefix}{suffix}");
        }
        package.skills[index].name=candidate;
    }
    let changed=tx.execute("INSERT INTO skill_packages(id,package_key,name,source,version,license,builtin,digest,file_count,total_bytes,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![package.id,package.package_key,package.name,package.source,package.version,package.license,package.builtin,package.digest,package.files.len()as i64,package.total_bytes as i64,stamp()]).map_err(err)?;
    if changed!=1{return Err("skill_package_write_not_persisted".into());}
    for file in &package.files{
        let text=std::str::from_utf8(&file.bytes).is_ok()&&!file.bytes.contains(&0);
        let kind=resource_kind(&file.path);
        if tx.execute("INSERT INTO skill_package_files(package_id,path,content,size_bytes,sha256,text,kind) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![package.id,file.path,file.bytes,file.bytes.len()as i64,file.sha256,text,kind]).map_err(err)?!=1{return Err("skill_package_write_not_persisted".into());}
    }
    for skill in &package.skills{
        if tx.execute("INSERT INTO agent_skills(name,description,instructions,builtin,enabled) VALUES(?1,?2,?3,?4,1)",params![skill.name,skill.description,skill.instructions,package.builtin]).map_err(err)?!=1{return Err("skill_package_write_not_persisted".into());}
        let id=tx.last_insert_rowid();
        if tx.execute("INSERT INTO skill_package_entries(skill_id,package_id,entry_path) VALUES(?1,?2,?3)",params![id,package.id,skill.entry_path]).map_err(err)?!=1{return Err("skill_package_write_not_persisted".into());}
        let stored:String=tx.query_row("SELECT instructions FROM agent_skills WHERE id=?1",[id],|r|r.get(0)).map_err(err)?;
        if stored!=skill.instructions{return Err("skill_package_write_not_persisted".into());}
    }
    let saved=info(&tx,&package.id)?;
    let count:i64=tx.query_row("SELECT COUNT(*) FROM skill_package_files WHERE package_id=?1",[&package.id],|r|r.get(0)).map_err(err)?;
    if saved.skill_count!=package.skills.len()||count as usize!=package.files.len(){return Err("skill_package_write_not_persisted".into());}
    tx.commit().map_err(err)?;Ok(saved)
}

pub(super) fn set_enabled(db:&mut Connection,skill_id:i64,enabled:bool)->Result<(),String>{
    let tx=db.transaction_with_behavior(TransactionBehavior::Immediate).map_err(err)?;
    if tx.execute("UPDATE agent_skills SET enabled=?2,updated_at=?3 WHERE id=?1 AND EXISTS(SELECT 1 FROM skill_package_entries e WHERE e.skill_id=agent_skills.id)",params![skill_id,enabled,stamp()]).map_err(err)?!=1{return Err("packaged_skill_unavailable".into());}
    let saved:bool=tx.query_row("SELECT enabled FROM agent_skills WHERE id=?1",[skill_id],|r|r.get(0)).map_err(err)?;
    if saved!=enabled{return Err("skill_package_write_not_persisted".into());}
    tx.commit().map_err(err)
}

fn mapping(db:&Connection,skill_id:i64)->Result<(String,String),String>{
    db.query_row("SELECT package_id,entry_path FROM skill_package_entries WHERE skill_id=?1",[skill_id],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?.ok_or_else(||"packaged_skill_unavailable".into())
}
fn resource_kind(path:&str)->&'static str{
    if path=="SKILL.md"||path.ends_with("/SKILL.md"){"instructions"}
    else if path.split('/').any(|part|part=="scripts")||[".sh",".py",".rb",".js",".ts",".ps1",".bat",".cmd"].iter().any(|ext|path.ends_with(ext)){"script"}
    else if path.split('/').any(|part|part=="references"){"reference"}else{"resource"}
}
fn allowed_resource(entry:&str,path:&str)->bool{
    let prefix=entry.strip_suffix("SKILL.md").unwrap_or("");
    path.starts_with(prefix)&&(!prefix.is_empty()||!path.starts_with("skills/"))||["LICENSE","LICENSE.md","LICENSE.txt","NOTICE"].contains(&path)
}
fn private_resource(path:&str)->bool{
    path.split('/').any(|part|{
        let name=part.to_ascii_lowercase();
        name==".env"||name.starts_with(".env.")||[".ssh",".aws",".npmrc",".netrc",".git-credentials","credentials","credentials.json","secrets.json","auth.json","id_rsa","id_ed25519","cookies.sqlite"].contains(&name.as_str())||[".pem",".p12",".pfx",".key",".keychain"].iter().any(|suffix|name.ends_with(suffix))
    })
}
fn sensitive_text(text:&str)->bool{
    static PATTERN:std::sync::OnceLock<regex::Regex>=std::sync::OnceLock::new();
    let lower=text.to_ascii_lowercase();
    if ["-----begin private key-----","-----begin rsa private key-----","-----begin openssh private key-----","-----begin ec private key-----"].iter().any(|mark|lower.contains(mark)){return true;}
    PATTERN.get_or_init(||regex::Regex::new(r#"(?i)(?:\bAKIA[0-9A-Z]{16}\b|\b(?:sk-(?:proj-)?|ghp_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{12,}|(?:password|api[_-]?key|access[_-]?token|secret[_-]?key)\s*[\"']?\s*[:=]\s*[\"'][^\"'\r\n]{6,}[\"'])"#).expect("constant credential pattern")).is_match(text)
}

pub(super) fn list_resources(db:&Connection,skill_id:i64)->Result<Value,String>{
    let (package_id,entry)=mapping(db,skill_id)?;
    let mut statement=db.prepare("SELECT path,size_bytes,sha256,text,kind FROM skill_package_files WHERE package_id=?1 ORDER BY path LIMIT 513").map_err(err)?;
    let rows=statement.query_map([&package_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    if rows.len()>512{return Err("skill_package_file_limit".into());}
    let resources:Vec<_>=rows.into_iter().filter(|(path,..)|allowed_resource(&entry,path)&&!private_resource(path)).map(|(path,size,sha,text,kind)|json!({"path":path,"sizeBytes":size,"sha256":sha,"text":text,"kind":kind})).collect();
    Ok(json!({"skillId":skill_id,"packageId":package_id,"resources":resources}))
}

pub(super) fn read_resource(db:&Connection,skill_id:i64,path:&str,offset:usize,limit:usize)->Result<Value,String>{
    import::checked_path(path)?;
    if limit==0||limit>8192{return Err("skill_package_page_limit".into());}
    let (package_id,entry)=mapping(db,skill_id)?;
    if !allowed_resource(&entry,path)||private_resource(path){return Err("skill_package_resource_denied".into());}
    let value:Option<(Vec<u8>,String)>=db.query_row("SELECT content,sha256 FROM skill_package_files WHERE package_id=?1 AND path=?2 AND size_bytes<=1048576",params![package_id,path],|r|Ok((r.get(0)?,r.get(1)?))).optional().map_err(err)?;
    let (bytes,hash)=value.ok_or("skill_package_resource_unavailable")?;
    if bytes.len()>import::MAX_FILE_BYTES||import::hash(&bytes)!=hash{return Err("skill_package_resource_integrity_failed".into());}
    if offset>bytes.len(){return Err("skill_package_offset_invalid".into());}
    let text=std::str::from_utf8(&bytes).ok().filter(|_|!bytes.contains(&0));
    let Some(text)=text else{return Ok(json!({"skillId":skill_id,"path":path,"content":null,"encoding":"binary","offset":0,"nextOffset":null,"hasMore":false,"totalBytes":bytes.len()}));};
    // Scan the complete bounded resource before returning any page, so splitting
    // a credential across page boundaries cannot evade disclosure protection.
    if sensitive_text(text){return Err("skill_package_credential_content_denied".into());}
    if !text.is_char_boundary(offset){return Err("skill_package_offset_invalid".into());}
    let mut end=offset.saturating_add(limit).min(text.len());
    while end>offset&&!text.is_char_boundary(end){end-=1;}
    if end==offset&&offset<text.len(){return Err("skill_package_page_too_small".into());}
    let more=end<text.len();
    Ok(json!({"skillId":skill_id,"path":path,"content":&text[offset..end],"encoding":"utf8","offset":offset,"nextOffset":if more{Some(end)}else{None},"hasMore":more,"totalBytes":text.len()}))
}

pub(super) fn package_fingerprint(db:&Connection,skill_id:i64)->Result<Option<String>,String>{
    // A stored package digest describes the imported snapshot, but is not proof
    // that the rows are still that snapshot. Bind consent to current provenance,
    // the selected entry and the actual bounded bytes of its accessible files.
    let package:Option<(String,String,Value)>=db.query_row(
        "SELECT p.id,e.entry_path,p.package_key,p.name,p.source,p.version,p.license,p.builtin,p.digest,p.file_count,p.total_bytes FROM skill_package_entries e JOIN skill_packages p ON p.id=e.package_id WHERE e.skill_id=?1",
        [skill_id],|r|Ok((r.get(0)?,r.get(1)?,json!({
            "packageKey":r.get::<_,String>(2)?,"name":r.get::<_,String>(3)?,
            "source":r.get::<_,String>(4)?,"version":r.get::<_,String>(5)?,
            "license":r.get::<_,String>(6)?,"builtin":r.get::<_,bool>(7)?,
            "digest":r.get::<_,String>(8)?,"fileCount":r.get::<_,i64>(9)?,
            "totalBytes":r.get::<_,i64>(10)?,
        }))),
    ).optional().map_err(err)?;
    let Some((package_id,entry,metadata))=package else{return Ok(None);};
    import::checked_path(&entry)?;
    let mut statement=db.prepare("SELECT path,size_bytes,sha256,text,kind FROM skill_package_files WHERE package_id=?1 ORDER BY path LIMIT 513").map_err(err)?;
    let rows=statement.query_map([&package_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,String>(4)?))).map_err(err)?.collect::<Result<Vec<_>,_>>().map_err(err)?;
    if rows.len()>import::MAX_FILES{return Err("skill_package_file_limit".into());}
    let mut files=Vec::new();let mut total=0usize;let mut found_entry=false;
    for (path,size,expected,text,kind) in rows {
        import::checked_path(&path)?;
        if !allowed_resource(&entry,&path){continue;}
        if size<0||size as usize>import::MAX_FILE_BYTES{return Err("skill_package_resource_integrity_failed".into());}
        let bytes:Option<Vec<u8>>=db.query_row("SELECT content FROM skill_package_files WHERE package_id=?1 AND path=?2 AND length(content)<=1048576",params![package_id,path],|r|r.get(0)).optional().map_err(err)?;
        let bytes=bytes.ok_or("skill_package_resource_integrity_failed")?;
        let actual=import::hash(&bytes);
        if bytes.len()!=size as usize||actual!=expected{return Err("skill_package_resource_integrity_failed".into());}
        total=total.checked_add(bytes.len()).ok_or("skill_package_byte_limit")?;
        if total>import::MAX_TOTAL_BYTES{return Err("skill_package_byte_limit".into());}
        found_entry|=path==entry;
        files.push(json!({"path":path,"size":size,"sha256":actual,"text":text,"kind":kind}));
    }
    if !found_entry{return Err("skill_package_resource_integrity_failed".into());}
    Ok(Some(import::hash(json!({"packageId":package_id,"entry":entry,"metadata":metadata,"files":files}).to_string().as_bytes())))
}
