//! Seatbelt platform evidence, not an approved-package production runner.
//! A successful fixture proves only the guarantees it actually exercises.
//! Hard per-run memory containment remains unverified; no execution API is exposed.

pub(crate) struct MacosStaticParserReadiness {
    mechanism_available: bool,
}
impl MacosStaticParserReadiness {
    pub(crate) fn inspect() -> Self {
        #[cfg(target_os = "macos")]
        let available = {
            use std::os::unix::fs::MetadataExt;
            std::fs::symlink_metadata("/usr/bin/sandbox-exec")
                .is_ok_and(|m| m.is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
        };
        #[cfg(not(target_os = "macos"))]
        let available = false;
        Self {
            mechanism_available: available,
        }
    }
    pub(crate) fn require_supported(&self) -> Result<(), &'static str> {
        if !self.mechanism_available {
            return Err("unsupported_sandbox:macos_seatbelt_unavailable");
        }
        // RLIMIT_CPU is CPU time, not a core quota. RLIMIT_RSS/AS must not
        // silently stand in for an unproved hard per-run memory boundary.
        Err("unsupported_sandbox:macos_hard_memory_containment_unverified")
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::MacosStaticParserReadiness;
    use std::{
        fs,
        io::Read,
        os::unix::{fs::PermissionsExt, process::CommandExt},
        path::PathBuf,
        process::{Child, Command, Stdio},
        sync::atomic::{AtomicU64, Ordering},
        time::{Duration, Instant},
    };
    static NEXT: AtomicU64 = AtomicU64::new(0);
    // Only this fixed compiled probe runs. The fixture cannot accept caller code,
    // a program path, arguments, a supplied Seatbelt profile, or network targets.
    const SOURCE: &str = r#"
#include <stdio.h>
#include <string.h>
#include <errno.h>
#include <fcntl.h>
#include <unistd.h>
#include <sys/socket.h>
#include <netinet/in.h>
int main(int argc,char **argv){
 if(argc<2)return 2;
 if(!strcmp(argv[1],"files")){
  if(argc!=6)return 2;
  for(int i=2;i<6;i++){errno=0;int fd=open(argv[i],i==3||i==5?O_WRONLY|O_CREAT:O_RDONLY,0600);int e=errno;printf("file%d=%d errno=%d\n",i-1,fd>=0,e);if(fd>=0)close(fd);}
  int s=socket(AF_INET,SOCK_STREAM,0);if(s<0)return 3;
  struct sockaddr_in a={0};a.sin_family=AF_INET;a.sin_port=htons(9);a.sin_addr.s_addr=htonl(0x7f000001);
  errno=0;int result=connect(s,(struct sockaddr*)&a,sizeof(a));printf("connect=%d errno=%d\n",result,errno);close(s);return 0;
 }
 if(!strcmp(argv[1],"fork")){errno=0;int p=fork();printf("fork=%d errno=%d\n",p,errno);if(p==0)_exit(4);errno=0;int s=setsid();printf("setsid=%d errno=%d\n",s>=0,errno);return p<0?0:4;}
 if(!strcmp(argv[1],"cpu")){volatile unsigned long n=0;for(;;)n++;}
 if(!strcmp(argv[1],"output")){char b[1024];memset(b,'x',sizeof b);for(;;)if(write(1,b,sizeof b)<0)return 3;}
 if(!strcmp(argv[1],"cancel")){write(1,"ready\n",6);for(;;)sleep(1);}
 return 2;
}
"#;
    enum Mode {
        Files,
        Fork,
        Cpu,
        Output,
        Cancel,
    }
    impl Mode {
        fn name(&self) -> &'static str {
            match self {
                Self::Files => "files",
                Self::Fork => "fork",
                Self::Cpu => "cpu",
                Self::Output => "output",
                Self::Cancel => "cancel",
            }
        }
    }
    struct Fixture {
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root = PathBuf::from(format!(
                "/tmp/oviraptor-seatbelt-fixed-probe-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
            for p in ["input", "output", "outside"] {
                fs::create_dir(root.join(p)).unwrap();
            }
            fs::write(root.join("input/evidence"), b"owned frozen input").unwrap();
            fs::write(root.join("outside/evidence"), b"owned outside input").unwrap();
            fs::write(root.join("probe.c"), SOURCE).unwrap();
            let root = root.canonicalize().unwrap();
            let built = Command::new("/usr/bin/cc")
                .arg(root.join("probe.c"))
                .arg("-o")
                .arg(root.join("probe"))
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "fixed test probe compilation failed: {}",
                String::from_utf8_lossy(&built.stderr)
            );
            let path = root.to_str().unwrap();
            assert!(
                path.starts_with("/private/tmp/oviraptor-seatbelt-fixed-probe-")
                    && !path.contains(['"', '\\', '\n'])
            );
            // Never grant /System/Volumes/Data via a broad /System subpath.
            // Literal / is needed by dyld; it does not grant arbitrary children.
            let profile = format!(
                r#"(version 1)
(deny default)
(allow process-exec (literal "{path}/probe"))
(allow file-read* (literal "/") (subpath "/System/Library") (subpath "/usr/lib") (subpath "/System/Volumes/Preboot/Cryptexes/OS/System/Library") (subpath "/System/Volumes/Preboot/Cryptexes/OS/usr/lib") (literal "/dev/null") (literal "{path}/probe") (subpath "{path}/input"))
(allow file-write* (subpath "{path}/output") (literal "/dev/null"))
"#
            );
            fs::write(root.join("profile.sb"), profile).unwrap();
            Self { root }
        }
        fn launch(&self, mode: Mode) -> OwnedChild {
            let mut cmd = Command::new("/usr/bin/sandbox-exec");
            cmd.arg("-f")
                .arg(self.root.join("profile.sb"))
                .arg(self.root.join("probe"))
                .arg(mode.name())
                .env_clear()
                .current_dir(self.root.join("output"))
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if matches!(mode, Mode::Files) {
                for p in [
                    "input/evidence",
                    "output/result",
                    "outside/evidence",
                    "outside/result",
                ] {
                    cmd.arg(self.root.join(p));
                }
            }
            if matches!(mode, Mode::Cpu) {
                // Only async-signal-safe libc is used between fork and exec.
                unsafe {
                    cmd.pre_exec(|| {
                        let limit = libc::rlimit {
                            rlim_cur: 1,
                            rlim_max: 1,
                        };
                        if libc::setrlimit(libc::RLIMIT_CPU, &limit) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                        Ok(())
                    });
                }
            }
            OwnedChild(cmd.spawn().unwrap())
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.root).unwrap();
        }
    }
    struct OwnedChild(Child);
    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn wait(child: &mut OwnedChild, limit: Duration) -> std::process::ExitStatus {
        let started = Instant::now();
        loop {
            if let Some(s) = child.0.try_wait().unwrap() {
                return s;
            }
            assert!(
                started.elapsed() < limit,
                "fixed probe exceeded test deadline"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn output(child: &mut OwnedChild) -> String {
        let mut s = String::new();
        child
            .0
            .stdout
            .as_mut()
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    }
    fn read_exact_bounded(child: &mut OwnedChild, target: &mut [u8]) {
        use std::os::fd::AsRawFd;
        let pipe = child.0.stdout.as_mut().unwrap();
        let fd = pipe.as_raw_fd();
        // This descriptor belongs to this disposable probe, never caller IO.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        let started = Instant::now();
        let mut offset = 0;
        while offset < target.len() {
            match pipe.read(&mut target[offset..]) {
                Ok(0) => panic!("fixed probe exited before expected output"),
                Ok(n) => offset += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(3),
                        "fixed probe output deadline exceeded"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("fixed probe pipe failed: {e}"),
            }
        }
    }
    #[test]
    fn macos_static_parser_deny_default_real_files_network_and_fork() {
        assert_eq!(
            MacosStaticParserReadiness::inspect()
                .require_supported()
                .unwrap_err(),
            "unsupported_sandbox:macos_hard_memory_containment_unverified"
        );
        let f = Fixture::new();
        let mut child = f.launch(Mode::Files);
        assert!(wait(&mut child, Duration::from_secs(3)).success());
        let s = output(&mut child);
        assert!(
            s.contains("file1=1 errno=0") && s.contains("file2=1 errno=0"),
            "{s}"
        );
        assert!(
            s.contains("file3=0 errno=1")
                && s.contains("file4=0 errno=1")
                && s.contains("connect=-1 errno=1"),
            "{s}"
        );
        assert!(!f.root.join("outside/result").exists());
        let mut child = f.launch(Mode::Fork);
        assert!(wait(&mut child, Duration::from_secs(3)).success());
        let s = output(&mut child);
        assert!(s.contains("fork=-1 errno=1"), "{s}");
        // setsid may succeed. No-fork makes direct-PID kill/reap the lifecycle
        // boundary; this test never claims process-group containment instead.
        assert!(s.contains("setsid="), "{s}");
    }
    #[test]
    fn macos_static_parser_real_cpu_output_and_cancel_are_bounded_and_reaped() {
        use std::os::unix::process::ExitStatusExt;
        let f = Fixture::new();
        let mut child = f.launch(Mode::Cpu);
        let status = wait(&mut child, Duration::from_secs(4));
        assert!(
            matches!(status.signal(), Some(libc::SIGXCPU) | Some(libc::SIGKILL)),
            "{status}"
        );
        let mut child = f.launch(Mode::Output);
        let mut bytes = [0; 4096];
        read_exact_bounded(&mut child, &mut bytes);
        assert!(bytes.iter().all(|b| *b == b'x'));
        child.0.kill().unwrap();
        assert_eq!(
            wait(&mut child, Duration::from_secs(2)).signal(),
            Some(libc::SIGKILL)
        );
        assert!(child.0.try_wait().unwrap().is_some());
        let mut child = f.launch(Mode::Cancel);
        let mut ready = [0; 6];
        read_exact_bounded(&mut child, &mut ready);
        assert_eq!(&ready, b"ready\n");
        child.0.kill().unwrap();
        assert_eq!(
            wait(&mut child, Duration::from_secs(2)).signal(),
            Some(libc::SIGKILL)
        );
        assert!(child.0.try_wait().unwrap().is_some());
    }
}
