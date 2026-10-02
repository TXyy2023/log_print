"""Rebuild the offline frontend from its lock, using the platform npm launcher."""
import shutil
import subprocess
from pathlib import Path
root=Path(__file__).resolve().parents[2]
frontend=root/'project/plugins/outputs/output-webui/frontend'
npm=shutil.which('npm.cmd' if __import__('os').name=='nt' else 'npm')
if not npm:raise SystemExit('Node/npm is required for frontend validation; Rust/runtime uses embedded assets.')
for args in [['ci'],['run','build']]:subprocess.run([npm,*args],cwd=frontend,check=True)
