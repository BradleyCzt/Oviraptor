use super::model::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::{collections::{BTreeMap, BTreeSet}, fs::{self, File}, io::Read, path::{Component, Path, PathBuf}};

pub(super) const MAX_FILES: usize = 512;
pub(super) const MAX_FILE_BYTES: usize = 1_048_576;
pub(super) const MAX_TOTAL_BYTES: usize = 16_777_216;
const MAX_MANIFEST_BYTES: usize = 25_165_824;

pub(super) fn hash(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
pub(super) fn checked_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 512 || path.contains('\\') || path.contains(':') || path.chars().any(char::is_control)
        || path.split('/').any(|part| part.is_empty() || part == "." || part == ".." || part.ends_with([' ', '.']))
        || Path::new(path).components().any(|part| !matches!(part, Component::Normal(_))) {
        return Err("skill_package_unsafe_path".into());
    }
    Ok(())
}

fn scalar(value: &str) -> String {
    let value = value.trim();
    if value.starts_with('"') && value.ends_with('"') {
        return serde_json::from_str::<String>(value).unwrap_or_default();
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return value[1..value.len()-1].replace("''", "'");
    }
    if value.starts_with(['!', '&', '*', '{', '[']) { return String::new(); }
    value.to_string()
}

fn metadata(instructions: &str, entry: &str) -> Result<(String, String), String> {
    if instructions.trim().is_empty() || instructions.contains('\0') || instructions.chars().count() > 30_000 {
        return Err(format!("skill_package_invalid_instructions:{entry}"));
    }
    let mut name = String::new(); let mut description = String::new();
    let mut lines = instructions.trim_start_matches('\u{feff}').lines().peekable();
    if lines.next().map(str::trim) == Some("---") {
        let mut closed = false;
        while let Some(line) = lines.next() {
            if line.trim() == "---" { closed = true; break; }
            let Some((key, value)) = line.split_once(':') else { continue; };
            if key != "name" && key != "description" { continue; }
            let text = if matches!(value.trim(), ">" | ">-" | "|" | "|-") {
                let mut text = Vec::new();
                while lines.peek().is_some_and(|line| line.starts_with([' ', '\t']) || line.is_empty()) {
                    text.push(lines.next().unwrap_or_default().trim().to_string());
                }
                text.join(if value.contains('>') { " " } else { "\n" }).trim().to_string()
            } else { scalar(value) };
            if key == "name" { name = text; } else { description = text; }
        }
        if !closed { return Err(format!("skill_package_invalid_frontmatter:{entry}")); }
    }
    if name.is_empty() {
        name = Path::new(entry).parent().and_then(Path::file_name).and_then(|v|v.to_str()).unwrap_or("custom-skill").to_string();
    }
    if name.trim().is_empty() || name.chars().count()>80 || name.chars().any(char::is_control) || description.len()>8192 {
        return Err(format!("skill_package_invalid_metadata:{entry}"));
    }
    Ok((name.trim().to_string(), description))
}

fn validate(mut package: Package) -> Result<Package, String> {
    if package.files.is_empty() || package.files.len()>MAX_FILES { return Err("skill_package_file_limit".into()); }
    package.files.sort_by(|a,b| a.path.cmp(&b.path));
    let mut paths=BTreeSet::new(); let mut names=BTreeSet::new(); let mut total=0usize;
    let mut hasher=Sha256::new();
    for value in [&package.package_key,&package.name,&package.version,&package.source,&package.license] {
        if value.len()>2048 || value.chars().any(char::is_control) { return Err("skill_package_invalid_manifest".into()); }
        hasher.update(value.as_bytes()); hasher.update([0]);
    }
    for file in &package.files {
        checked_path(&file.path)?;
        if !paths.insert(file.path.to_lowercase()) { return Err("skill_package_duplicate_path".into()); }
        if file.bytes.len()>MAX_FILE_BYTES { return Err(format!("skill_package_file_too_large:{}",file.path)); }
        total=total.checked_add(file.bytes.len()).ok_or("skill_package_byte_limit")?;
        if total>MAX_TOTAL_BYTES { return Err("skill_package_byte_limit".into()); }
        if hash(&file.bytes)!=file.sha256 { return Err(format!("skill_package_hash_mismatch:{}",file.path)); }
        hasher.update(file.path.as_bytes()); hasher.update([0]); hasher.update(file.sha256.as_bytes()); hasher.update([0]);
        if file.path=="SKILL.md" || file.path.ends_with("/SKILL.md") {
            let instructions=String::from_utf8(file.bytes.clone()).map_err(|_|format!("skill_package_non_utf8_instructions:{}",file.path))?;
            let (name,description)=metadata(&instructions,&file.path)?;
            if !names.insert(name.to_lowercase()) { return Err("skill_package_duplicate_skill_name".into()); }
            package.skills.push(PackageSkill{name,description,entry_path:file.path.clone(),instructions});
        }
    }
    if package.skills.is_empty() || package.skills.len()>128 { return Err("skill_package_no_skills_or_limit".into()); }
    package.total_bytes=total; package.digest=format!("{:x}",hasher.finalize());
    package.id=if package.builtin { package.package_key.clone() } else { format!("local-{}",&package.digest[..32]) };
    Ok(package)
}

pub(super) fn bundled() -> Result<Package,String> {
    let data=include_bytes!("../../resources/skill-packages/compound-engineering.json.gz");
    let mut bytes=Vec::new();
    flate2::read::GzDecoder::new(&data[..]).take((MAX_MANIFEST_BYTES+1) as u64).read_to_end(&mut bytes).map_err(|_|"skill_package_bundle_decode")?;
    if bytes.len()>MAX_MANIFEST_BYTES { return Err("skill_package_manifest_limit".into()); }
    let manifest:Manifest=serde_json::from_slice(&bytes).map_err(|_|"skill_package_invalid_manifest")?;
    if manifest.schema_version!=1 || manifest.package_key!="every-compound-engineering" || manifest.files.len()>MAX_FILES {
        return Err("skill_package_invalid_manifest".into());
    }
    let mut files=Vec::new();
    for file in manifest.files {
        if file.content_base64.len()>MAX_FILE_BYTES*4/3+8 { return Err("skill_package_file_too_large".into()); }
        let bytes=STANDARD.decode(file.content_base64).map_err(|_|"skill_package_invalid_base64")?;
        files.push(PackageFile{path:file.path,bytes,sha256:file.sha256});
    }
    validate(Package{id:String::new(),package_key:manifest.package_key,name:manifest.name,version:manifest.version,source:manifest.source,license:manifest.license,builtin:true,digest:String::new(),files,skills:Vec::new(),total_bytes:0})
}

fn reject_link(path: &Path) -> Result<std::fs::Metadata,String> {
    let meta=fs::symlink_metadata(path).map_err(|_|"skill_package_path_unavailable")?;
    if meta.file_type().is_symlink() { return Err("skill_package_symlink_forbidden".into()); }
    #[cfg(windows)] {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 { return Err("skill_package_reparse_point_forbidden".into()); }
    }
    Ok(meta)
}

#[cfg(unix)]
fn read_file(root: &File, _root_path:&Path, relative:&str) -> Result<Vec<u8>,String> {
    use std::{ffi::CString, os::fd::{AsRawFd,FromRawFd}};
    let mut directory=root.try_clone().map_err(|_|"skill_package_read_failed")?;
    let parts:Vec<_>=relative.split('/').collect();
    for (index,part) in parts.iter().enumerate() {
        let name=CString::new(*part).map_err(|_|"skill_package_unsafe_path")?;
        let flags=libc::O_RDONLY|libc::O_CLOEXEC|libc::O_NOFOLLOW|if index+1<parts.len(){libc::O_DIRECTORY}else{libc::O_NONBLOCK};
        let fd=unsafe{libc::openat(directory.as_raw_fd(),name.as_ptr(),flags)};
        if fd<0 { return Err("skill_package_safe_open_failed".into()); }
        directory=unsafe{File::from_raw_fd(fd)};
    }
    if !directory.metadata().map_err(|_|"skill_package_read_failed")?.is_file() { return Err("skill_package_special_file_forbidden".into()); }
    let mut bytes=Vec::new(); directory.take((MAX_FILE_BYTES+1)as u64).read_to_end(&mut bytes).map_err(|_|"skill_package_read_failed")?;
    if bytes.len()>MAX_FILE_BYTES { return Err("skill_package_file_too_large".into()); }
    Ok(bytes)
}

#[cfg(windows)]
fn read_file(root:&File, root_path:&Path, relative:&str)->Result<Vec<u8>,String> {
    use std::{os::windows::{fs::OpenOptionsExt,io::AsRawHandle},ffi::OsString,os::windows::ffi::OsStringExt};
    #[link(name="kernel32")]
    extern "system" { fn GetFinalPathNameByHandleW(handle:*mut std::ffi::c_void,path:*mut u16,length:u32,flags:u32)->u32; }
    fn resolved(file:&File)->Result<PathBuf,String> {
        let mut buffer=vec![0u16;32768];
        let length=unsafe{GetFinalPathNameByHandleW(file.as_raw_handle(),buffer.as_mut_ptr(),buffer.len()as u32,0)};
        if length==0 || length as usize>=buffer.len(){return Err("skill_package_safe_open_failed".into());}
        Ok(PathBuf::from(OsString::from_wide(&buffer[..length as usize])))
    }
    let path=root_path.join(relative); let mut current=root_path.to_path_buf();
    for part in Path::new(relative).components(){current.push(part);reject_link(&current)?;}
    let file=std::fs::OpenOptions::new().read(true).custom_flags(0x00200000).open(&path).map_err(|_|"skill_package_safe_open_failed")?;
    if !file.metadata().map_err(|_|"skill_package_read_failed")?.is_file() || !resolved(&file)?.starts_with(resolved(root)?) { return Err("skill_package_safe_open_failed".into()); }
    let mut bytes=Vec::new();file.take((MAX_FILE_BYTES+1)as u64).read_to_end(&mut bytes).map_err(|_|"skill_package_read_failed")?;
    if bytes.len()>MAX_FILE_BYTES { return Err("skill_package_file_too_large".into()); } Ok(bytes)
}

pub(super) fn local(path:&Path)->Result<Package,String> {
    if !reject_link(path)?.is_dir(){return Err("skill_package_select_directory_zip_unsupported".into());}
    let root_path=fs::canonicalize(path).map_err(|_|"skill_package_path_unavailable")?;
    let mut options=std::fs::OpenOptions::new();options.read(true);
    #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt;options.custom_flags(libc::O_NOFOLLOW|libc::O_DIRECTORY|libc::O_CLOEXEC); }
    #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt;options.custom_flags(0x02000000|0x00200000); }
    let root=options.open(&root_path).map_err(|_|"skill_package_safe_open_failed")?;
    let mut paths=BTreeMap::<String,PathBuf>::new();
    fn collect(root:&Path,path:&Path,depth:usize,paths:&mut BTreeMap<String,PathBuf>)->Result<(),String>{
        if depth>16{return Err("skill_package_depth_limit".into());}
        let meta=reject_link(path)?;
        if meta.is_dir(){
            for entry in fs::read_dir(path).map_err(|_|"skill_package_read_failed")?{
                let entry=entry.map_err(|_|"skill_package_read_failed")?; collect(root,&entry.path(),depth+1,paths)?;
            }
        }else if meta.is_file(){
            let relative=path.strip_prefix(root).map_err(|_|"skill_package_unsafe_path")?.to_str().ok_or("skill_package_non_utf8_path")?.to_string();
            #[cfg(windows)]
            let relative=relative.replace('\\',"/");
            checked_path(&relative)?;
            if meta.len()>MAX_FILE_BYTES as u64{return Err("skill_package_file_too_large".into());}
            paths.insert(relative,path.to_path_buf());
            if paths.len()>MAX_FILES{return Err("skill_package_file_limit".into());}
        }else{return Err("skill_package_special_file_forbidden".into());} Ok(())
    }
    let skill_file=root_path.join("SKILL.md");let skills_dir=root_path.join("skills");
    if skill_file.exists(){collect(&root_path,&root_path,0,&mut paths)?;}
    else if skills_dir.exists(){
        collect(&root_path,&skills_dir,0,&mut paths)?;
    }else{
        for entry in fs::read_dir(&root_path).map_err(|_|"skill_package_read_failed")?{
            let p=entry.map_err(|_|"skill_package_read_failed")?.path();
            let meta=reject_link(&p)?;
            if meta.is_dir() && p.join("SKILL.md").exists(){collect(&root_path,&p,0,&mut paths)?;}
        }
    }
    // Root-level legal notices belong to every supported folder layout.
    // In particular, sibling skill directories must not silently lose licensing.
    for name in ["LICENSE","LICENSE.md","LICENSE.txt","NOTICE"]{let p=root_path.join(name);if p.exists(){collect(&root_path,&p,0,&mut paths)?;}}
    let mut files=Vec::new();let mut total=0;
    for (relative,_) in paths {let bytes=read_file(&root,&root_path,&relative)?;total+=bytes.len();if total>MAX_TOTAL_BYTES{return Err("skill_package_byte_limit".into());}files.push(PackageFile{path:relative,sha256:hash(&bytes),bytes});}
    let name=root_path.file_name().and_then(|v|v.to_str()).unwrap_or("Custom skills").to_string();
    validate(Package{id:String::new(),package_key:"local-folder".into(),name,version:"local-content".into(),source:"Local folder import".into(),license:"See package resources".into(),builtin:false,digest:String::new(),files,skills:Vec::new(),total_bytes:0})
}

pub(super) fn preview(package:&Package)->PackagePreview{
    PackagePreview{digest:package.digest.clone(),name:package.name.clone(),skill_count:package.skills.len(),file_count:package.files.len(),total_bytes:package.total_bytes,skills:package.skills.iter().map(|s|PreviewSkill{name:s.name.clone(),description:s.description.clone(),entry_path:s.entry_path.clone()}).collect(),missing_capabilities:capability_notices(),warnings:vec!["Existing skill names are never overwritten. Importing does not grant these skills to any conversation.".into()]}
}
