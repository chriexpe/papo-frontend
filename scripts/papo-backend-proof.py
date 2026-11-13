#!/usr/bin/env python3
"""Build the pinned backend and verify cursor semantics and the example on loopback."""
import argparse
import os
import pathlib
import shutil
import subprocess
import tempfile
import time
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--backend', type=pathlib.Path, required=True)
args = parser.parse_args()
backend = args.backend.resolve()
pin = (ROOT / 'crates/sdk/api/protocol/backend-commit').read_text().strip()
actual = subprocess.check_output(['git', '-C', str(backend), 'rev-parse', 'HEAD'], text=True).strip()
if actual != pin:
    raise SystemExit('Backend checkout does not match the protocol pin')
# This directory is ignored build output, using the one existing target directory.
proof = backend / 'backend/cmd/sdk-proof'
proof.mkdir(parents=True, exist_ok=True)
shutil.copyfile(ROOT / 'scripts/papo-backend-proof.go', proof / 'main.go')
shutil.copyfile(ROOT / 'scripts/papo-permission-proof.go', proof / 'permissions.go')
subprocess.run(['go', 'build', '-o', str(ROOT / 'target/papo-backend-pinned'), './cmd/main.go'], cwd=backend / 'backend', check=True)
subprocess.run(['go', 'build', '-o', str(ROOT / 'target/papo-backend-proof'), './cmd/sdk-proof'], cwd=backend / 'backend', check=True)
with tempfile.TemporaryDirectory(prefix='papo-backend-proof-') as directory:
    address_file = pathlib.Path(directory) / 'address'
    env = dict(os.environ, PAPO_PROOF_ADDRESS_FILE=str(address_file))
    with subprocess.Popen([str(ROOT / 'target/papo-backend-proof')], cwd=backend / 'backend', env=env,
                          stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) as process:
        try:
            for _ in range(300):
                if process.poll() is not None:
                    raise RuntimeError('Backend exited before readiness')
                if address_file.exists():
                    address = address_file.read_text()
                    try:
                        with urllib.request.urlopen(address + '/health', timeout=1):
                            break
                    except OSError:
                        pass
                time.sleep(0.1)
            else:
                raise RuntimeError('Backend readiness timed out')
            env['PAPO_BACKEND_PROOF_ADDRESS'] = address
            subprocess.run(['cargo', 'test', '--locked', '-p', 'papo-api', '--test', 'postgres', '--', '--ignored'], cwd=ROOT, env=env, check=True)
            subprocess.run(['cargo', 'test', '--locked', '-p', 'papo-sdk', '--features', 'test-support', '--test', 'session12_backend', '--', '--ignored'], cwd=ROOT, env=env, check=True)
            subprocess.run(['cargo', 'test', '--locked', '-p', 'papo-sdk-ui', '--test', 'permission_backend', '--', '--ignored'], cwd=ROOT, env=env, check=True)
            env.pop('PAPO_SERVER_PASSWORD', None)
            env.update(PAPO_ADDRESS=address, PAPO_USERNAME='cursorproof', PAPO_PASSWORD=env['PAPO_PROOF_PASSWORD'],
                       PAPO_LOOPBACK_HTTP='1', PAPO_EVENT_LIMIT='1')
            subprocess.run(['cargo', 'run', '--locked', '-p', 'papo-api', '--example', 'live_events'], cwd=ROOT, env=env, check=True)
            env['PAPO_CHANNEL_NAME'] = 'upload-proof'
            subprocess.run(['cargo', 'run', '--locked', '-p', 'papo-sdk', '--features', 'test-support', '--example', 'outgoing'], cwd=ROOT, env=env, check=True)
        finally:
            process.terminate()
            process.wait(timeout=10)
