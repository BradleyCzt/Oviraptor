"""Actual fixed-container guardian tests. No App UI, database or model calls."""
import json
import os
from pathlib import Path
import select
import signal
import subprocess
import sys
import tempfile
import time

BINARY = Path(sys.argv[1]).resolve()
DOCKER = '/Applications/Docker.app/Contents/Resources/bin/docker'
ENV = {'HOME': os.environ['HOME'], 'TMPDIR': tempfile.gettempdir()}
RUNTIME = json.loads(subprocess.check_output(
    [str(BINARY), '--nest-private-parser-runtime-v1'], env=ENV, text=True, timeout=20))
ARGUMENT = '--nest-private-parser-guardian-v1'
LABEL = 'com.oviraptor.offline-parser-owner'


def request(code):
    return {'code': code, 'input': {'Cookie': 'session=fixture-account-a',
            'Token': 'fixture-account-b', 'password': 'fixture-password'},
            'limits': {'memoryMib': 32, 'timeoutSeconds': 10, 'outputBytes': 4096},
            'runtimeDigest': RUNTIME['runtimeDigest']}


def owned_ids():
    output = subprocess.check_output([DOCKER, 'container', 'ls', '-a', '--filter',
                                     'label=' + LABEL, '--format', '{{.ID}}'], text=True, timeout=5)
    return set(output.split())


def wait_new(before):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        added = owned_ids() - before
        if added:
            assert len(added) == 1, 'unexpected concurrent test container'
            identifier = added.pop()
            value = json.loads(subprocess.check_output(
                [DOCKER, 'container', 'inspect', identifier], text=True, timeout=5))[0]
            if value['State']['Running']:
                return identifier
        time.sleep(0.05)
    raise AssertionError('guardian did not start its own container')


def read_receipt(process):
    ready, _, _ = select.select([process.stdout], [], [], 20)
    assert ready, 'guardian exceeded cleanup deadline'
    result = json.loads(process.stdout.readline())
    assert process.wait(timeout=5) == 0
    assert result['cleanupVerified']
    return result


before = owned_ids()
positive = subprocess.Popen([str(BINARY), ARGUMENT], env=ENV,
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
positive.stdin.write((json.dumps(request('def analyze(data):\n return data')) + '\n').encode())
positive.stdin.flush()
receipt = read_receipt(positive)
positive.stdin.close()
assert receipt['exitCode'] == 0, receipt['stderr']
result = json.loads(receipt['stdout'])
assert result['result'] == request('')['input']
assert result['codeSha256'] == receipt['codeSha256']
assert result['inputSha256'] == receipt['inputSha256']
assert owned_ids() == before
print('PASS: actual guardian executes frozen input and verifies cleanup', flush=True)

cancelled = subprocess.Popen([str(BINARY), ARGUMENT], env=ENV,
                             stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
cancelled.stdin.write((json.dumps(request('def analyze(data):\n while True: pass')) + '\n').encode())
cancelled.stdin.flush()
identifier = wait_new(before)
cancelled.stdin.close()
receipt = read_receipt(cancelled)
assert receipt['stopReason'] == 'cancelled'
assert owned_ids() == before
print('PASS: closed parent pipe kills and removes the exact owned container', flush=True)

# The pipe writer belongs solely to this disposable parent. Killing it really
# exercises EOF across processes; no App, real user process or DB is touched.
helper = r'''
import json,subprocess,sys,time
child=subprocess.Popen([sys.argv[1],sys.argv[2]],stdin=subprocess.PIPE,
                       stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
child.stdin.write((sys.argv[3]+'\n').encode());child.stdin.flush()
print(child.pid,flush=True)
while True: time.sleep(1)
'''
parent = subprocess.Popen([sys.executable, '-c', helper, str(BINARY), ARGUMENT,
                           json.dumps(request('def analyze(data):\n while True: pass'))],
                          env=ENV, stdout=subprocess.PIPE, text=True)
guardian_pid = int(parent.stdout.readline())
identifier = wait_new(before)
parent.kill()
assert parent.wait(timeout=5) == -signal.SIGKILL
deadline = time.monotonic() + 10
while time.monotonic() < deadline and identifier in owned_ids():
    time.sleep(0.05)
assert identifier not in owned_ids(), 'parent SIGKILL left an uncontrolled container'
assert owned_ids() == before
print('PASS: actual parent SIGKILL is cleaned by an independent guardian', flush=True)

orphan = subprocess.Popen([str(BINARY), ARGUMENT], env=ENV,
                         stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
orphan.stdin.write((json.dumps(request('def analyze(data):\n while True: pass')) + '\n').encode())
orphan.stdin.flush()
identifier = wait_new(before)
orphan.kill()
assert orphan.wait(timeout=5) == -signal.SIGKILL
orphan.stdin.close()
assert identifier in owned_ids(), 'the actual guardian-kill fault must leave recoverable work'
recovery = json.loads(subprocess.check_output(
    [str(BINARY), '--nest-private-parser-cleanup-v1'], env=ENV, text=True, timeout=20))
assert any(row['containerId'].startswith(identifier) and row['cleanupVerified']
           and row['executionReplayed'] is False for row in recovery['recovered'])
assert owned_ids() == before
print('PASS: killed guardian is recovered from its locked durable journal without replay', flush=True)
print(json.dumps({'tests': 4, 'passed': 4, 'runtime': RUNTIME,
                  'databaseAccess': False, 'modelCalls': 0}), flush=True)
