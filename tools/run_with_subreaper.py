#!/usr/bin/env python3
"""Run Linux tests with init-like orphan reaping in a container.

A container whose PID 1 does not reap adopted children leaves killed process
members as zombies. That invalidates process-group cleanup tests even though
normal Linux init would reap them. This wrapper only adopts/reaps orphans; the
application still owns its process groups and all cleanup assertions stay active.

Example (after configuring the native build environment):
    python3 tools/run_with_subreaper.py cargo test \
        --manifest-path src-tauri/Cargo.toml --locked --lib -- --test-threads=2
"""
import ctypes
import os
import subprocess
import sys
import time

if sys.platform != 'linux':
    raise SystemExit('run_with_subreaper.py requires Linux PR_SET_CHILD_SUBREAPER')
if len(sys.argv) < 2:
    raise SystemExit('usage: run-with-subreaper.py COMMAND [ARGS...]')
libc = ctypes.CDLL(None, use_errno=True)
if libc.prctl(36, 1, 0, 0, 0) != 0:  # Linux PR_SET_CHILD_SUBREAPER
    raise OSError(ctypes.get_errno(), 'PR_SET_CHILD_SUBREAPER')
main = subprocess.Popen(sys.argv[1:])
main_status = None
reaped_descendants = 0
while True:
    try:
        pid, status = os.waitpid(-1, os.WNOHANG)
    except ChildProcessError:
        if main_status is not None:
            break
        raise
    if pid == main.pid:
        main_status = os.waitstatus_to_exitcode(status)
        main.returncode = main_status
    elif pid:
        reaped_descendants += 1
    else:
        time.sleep(0.005)
print(f'subreaper: reaped {reaped_descendants} orphan descendants', file=sys.stderr)
raise SystemExit(main_status if main_status is not None and main_status >= 0 else 128 - (main_status or 0))
