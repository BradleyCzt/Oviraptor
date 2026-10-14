//! Descriptor-relative, no-follow read-only directory access. No path is opened
//! from model-controlled absolute input; Windows fails closed in this version.
use serde_json::{json, Value};
use std::{fs, io::Read, path::{Component, Path}};
const MAX_TEXT:usize=16_384;
const MAX_FILE:u64=1_048_576;
const MAX_ENTRIES:usize=64;
#[derive(Clone,Debug)]
pub(super) struct Root{pub path:String,pub device:String,pub inode:String,pub label:String}
fn denied_name(name:&str)->bool{
    let lower=name.to_ascii_lowercase();
    name.starts_with('.')||name.chars().any(char::is_control)||name.len()>255
    ||["node_modules","target","appdata","keychains","keychain","library","credentials","credential","secrets","secret","passwords","private","id_rsa","id_dsa","id_ed25519","known_hosts","authorized_keys","auth.json","config.json","cookies","cookies.sqlite","login data","local state"].contains(&lower.as_str())
    ||[".pem",".key",".p12",".pfx",".jks",".keystore",".sqlite",".sqlite3",".db"].iter().any(|s|lower.ends_with(s))
    ||lower.contains("credential")||lower.contains("password")||lower.contains("secret")
}
fn allowed_file(name:&str)->bool{
    if denied_name(name){return false;}
    let lower=name.to_ascii_lowercase();
    if ["readme","license","makefile","dockerfile","skill.md","cargo.lock","package-lock.json"].contains(&lower.as_str()){return true;}
    ["md","txt","rst","rs","ts","tsx","js","jsx","mjs","cjs","vue","json","toml","yaml","yml","py","java","go","c","h","cc","cpp","hpp","css","html","xml","sh","sql","csv","tsv"].contains(&lower.rsplit('.').next().unwrap_or(""))
}
fn relative(value:&str,allow_root:bool)->Result<Vec<String>,String>{
    if value.is_empty()||value=="." {return if allow_root{Ok(vec![])}else{Err("conversation_directory_file_required".into())};}
    if value.len()>1024||value.contains('\\')||value.chars().any(char::is_control){return Err("conversation_directory_invalid_path".into());}
    let mut parts=vec![];
    for part in Path::new(value).components(){match part{Component::Normal(name)=>{let name=name.to_str().ok_or("conversation_directory_path_encoding")?;if denied_name(name){return Err("conversation_directory_private_path_denied".into());}parts.push(name.into());},_=>return Err("conversation_directory_invalid_path".into())}}
    // Reject normalized-away separators/dot segments as well as traversal.
    if parts.len()>16||parts.join("/")!=value{return Err("conversation_directory_invalid_path".into());}Ok(parts)
}
fn sensitive_text(value:&str)->bool{
    let lower=value.to_ascii_lowercase();
    lower.contains("-----begin private key-----")||lower.contains("-----begin rsa private key-----")||lower.contains("-----begin openssh private key-----")||lower.contains("-----begin ec private key-----")
    || regex::Regex::new(r#"(?i)(?:\bAKIA[0-9A-Z]{16}\b|\b(?:sk-(?:proj-)?|ghp_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]{12,}|(?:password|api[_-]?key|access[_-]?token|secret[_-]?key)\s*[\"']?\s*[:=]\s*[\"'][^\"'\r\n]{6,}[\"'])"#).is_ok_and(|re|re.is_match(value))
}
pub(super) fn bounded_text(bytes:&[u8],limit:usize)->Result<(String,bool),String>{
    let truncated=bytes.len()>limit;let sample=&bytes[..bytes.len().min(limit)];
    let value=match std::str::from_utf8(sample){Ok(v)=>v,Err(e) if truncated&&e.error_len().is_none()=>std::str::from_utf8(&sample[..e.valid_up_to()]).map_err(|_|"conversation_directory_binary_denied")?,Err(_)=>return Err("conversation_directory_binary_denied".into())};
    if value.chars().any(|c|c=='\0'||(c.is_control()&&!['\n','\r','\t'].contains(&c))){return Err("conversation_directory_binary_denied".into());}
    if sensitive_text(value){return Err("conversation_directory_credential_content_denied".into());}Ok((value.into(),truncated))
}
#[cfg(unix)]
mod unix{
    use super::*;
    use std::{ffi::{CStr,CString},fs::File,os::{fd::{AsRawFd,FromRawFd,OwnedFd},unix::{ffi::OsStrExt,fs::MetadataExt}},path::PathBuf};
    fn open_at(fd:i32,name:&str,directory:bool)->Result<OwnedFd,String>{
        let name=CString::new(name).map_err(|_|"conversation_directory_invalid_path")?;
        let flags=libc::O_RDONLY|libc::O_CLOEXEC|libc::O_NOFOLLOW|libc::O_NONBLOCK|if directory{libc::O_DIRECTORY}else{0};
        let handle=unsafe{libc::openat(fd,name.as_ptr(),flags)};
        if handle<0{return Err("conversation_directory_unavailable_or_symlink".into());}
        Ok(unsafe{OwnedFd::from_raw_fd(handle)})
    }
    fn absolute_dir(path:&Path)->Result<OwnedFd,String>{
        if !path.is_absolute(){return Err("conversation_directory_absolute_root_required".into());}
        let mut fd=open_at(libc::AT_FDCWD,"/",true)?;
        for part in path.components(){match part{Component::RootDir=>{},Component::Normal(name)=>{let name=name.to_str().ok_or("conversation_directory_path_encoding")?;fd=open_at(fd.as_raw_fd(),name,true)?;},_=>return Err("conversation_directory_invalid_root".into())}}Ok(fd)
    }
    fn metadata(fd:&OwnedFd)->Result<fs::Metadata,String>{File::from(fd.try_clone().map_err(|_|"conversation_directory_unavailable")?).metadata().map_err(|_|"conversation_directory_unavailable".into())}
    pub(super) fn approve(path:&Path)->Result<Root,String>{
        if !path.is_absolute()||path.as_os_str().as_bytes().len()>4096{return Err("conversation_directory_invalid_root".into());}
        let given:PathBuf=path.components().collect();
        if path.components().any(|p|matches!(p,Component::ParentDir)){return Err("conversation_directory_invalid_root".into());}
        if fs::symlink_metadata(&given).map_err(|_|"conversation_directory_unavailable")?.file_type().is_symlink(){return Err("conversation_directory_symlink_root_denied".into());}
        let canonical=given.canonicalize().map_err(|_|"conversation_directory_unavailable")?;
        let text=canonical.to_str().ok_or("conversation_directory_path_encoding")?;
        let components:Vec<_>=canonical.components().filter_map(|c|if let Component::Normal(n)=c{n.to_str()}else{None}).collect();
        if components.len()<2||components.iter().enumerate().any(|(i,n)|denied_name(n)&&!(i==0&&*n=="private"&&(components.get(1)==Some(&"tmp")||(components.get(1)==Some(&"var")&&components.get(2)==Some(&"folders")))))||["/etc","/proc","/sys","/dev","/run","/boot","/System","/Library","/Windows","/Program Files","/private/etc"].iter().any(|p|canonical.starts_with(p)) {return Err("conversation_directory_private_root_denied".into());}
        if std::env::var_os("HOME").and_then(|h|PathBuf::from(h).canonicalize().ok()).is_some_and(|home|canonical==home){return Err("conversation_directory_home_root_denied".into());}
        // Resolve system aliases once, then every component is opened no-follow.
        let fd=absolute_dir(&canonical)?;let meta=metadata(&fd)?;
        Ok(Root{path:text.into(),device:meta.dev().to_string(),inode:meta.ino().to_string(),label:canonical.file_name().and_then(|v|v.to_str()).unwrap_or("Approved directory").chars().take(80).collect()})
    }
    fn root_fd(root:&Root)->Result<OwnedFd,String>{let fd=absolute_dir(Path::new(&root.path))?;let meta=metadata(&fd)?;if meta.dev().to_string()!=root.device||meta.ino().to_string()!=root.inode{return Err("conversation_directory_root_replaced".into());}Ok(fd)}
    fn descend(root:&Root,parts:&[String])->Result<OwnedFd,String>{let mut fd=root_fd(root)?;for part in parts{fd=open_at(fd.as_raw_fd(),part,true)?;}Ok(fd)}
    pub(super) fn read_file(root:&Root,path:&str)->Result<Value,String>{
        let parts=relative(path,false)?;let name=parts.last().ok_or("conversation_directory_file_required")?;
        if !allowed_file(name){return Err("conversation_directory_file_type_denied".into());}
        let parent=descend(root,&parts[..parts.len()-1])?;let fd=open_at(parent.as_raw_fd(),name,false)?;let meta=metadata(&fd)?;
        if !meta.is_file()||meta.nlink()!=1||meta.len()>MAX_FILE{return Err("conversation_directory_file_not_safe_regular_text".into());}
        let mut file=File::from(fd);let mut bytes=vec![];file.by_ref().take((MAX_TEXT+1)as u64).read_to_end(&mut bytes).map_err(|_|"conversation_directory_read_failed")?;
        let after=file.metadata().map_err(|_|"conversation_directory_read_failed")?;
        if after.nlink()!=1||after.dev()!=meta.dev()||after.ino()!=meta.ino()||after.len()!=meta.len()||after.mtime()!=meta.mtime()||after.mtime_nsec()!=meta.mtime_nsec(){return Err("conversation_directory_file_changed".into());}
        let(content,truncated)=bounded_text(&bytes,MAX_TEXT)?;
        Ok(json!({"untrusted":true,"rootLabel":root.label,"relativePath":path,"content":content,"truncated":truncated||meta.len()>MAX_TEXT as u64,"bytesRead":bytes.len().min(MAX_TEXT)}))
    }
    struct Directory(*mut libc::DIR);
    impl Drop for Directory{fn drop(&mut self){unsafe{libc::closedir(self.0);}}}
    pub(super) fn list_dir(root:&Root,path:&str)->Result<Value,String>{
        let parts=relative(path,true)?;let fd=descend(root,&parts)?;
        let copied=unsafe{libc::dup(fd.as_raw_fd())};if copied<0{return Err("conversation_directory_list_failed".into());}
        let pointer=unsafe{libc::fdopendir(copied)};if pointer.is_null(){unsafe{libc::close(copied)};return Err("conversation_directory_list_failed".into());}
        let dir=Directory(pointer);let mut entries=vec![];let mut seen=0;let mut truncated=false;
        loop{
            let entry=unsafe{libc::readdir(dir.0)};if entry.is_null(){break;}
            seen+=1;if seen>1024||entries.len()>=MAX_ENTRIES{truncated=true;break;}
            let name=unsafe{CStr::from_ptr((*entry).d_name.as_ptr())};let Ok(name)=name.to_str()else{continue};if denied_name(name){continue;}
            let c=CString::new(name).map_err(|_|"conversation_directory_invalid_path")?;let mut stat=std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe{libc::fstatat(fd.as_raw_fd(),c.as_ptr(),stat.as_mut_ptr(),libc::AT_SYMLINK_NOFOLLOW)}!=0{continue;}
            let stat=unsafe{stat.assume_init()};let kind=stat.st_mode&libc::S_IFMT;
            if kind==libc::S_IFDIR {entries.push(json!({"name":name,"kind":"directory"}));}
            else if kind==libc::S_IFREG&&stat.st_nlink==1&&stat.st_size>=0&&(stat.st_size as u64)<=MAX_FILE&&allowed_file(name){entries.push(json!({"name":name,"kind":"file","bytes":stat.st_size}));}
        }
        entries.sort_by(|a,b|a["name"].as_str().cmp(&b["name"].as_str()));
        Ok(json!({"untrusted":true,"rootLabel":root.label,"relativePath":path,"entries":entries,"truncated":truncated}))
    }
}
pub(super) fn approve_root(path:&Path)->Result<Root,String>{#[cfg(unix)]{unix::approve(path)}#[cfg(not(unix))]{let _=path;Err("conversation_directory_secure_access_unsupported".into())}}
pub(super) fn read(root:&Root,path:&str)->Result<Value,String>{#[cfg(unix)]{unix::read_file(root,path)}#[cfg(not(unix))]{let _=(root,path);Err("conversation_directory_secure_access_unsupported".into())}}
pub(super) fn list(root:&Root,path:&str)->Result<Value,String>{#[cfg(unix)]{unix::list_dir(root,path)}#[cfg(not(unix))]{let _=(root,path);Err("conversation_directory_secure_access_unsupported".into())}}
