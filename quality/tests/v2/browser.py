#!/usr/bin/env python3
"""Launch isolated native app/archive for the real-browser CI suite."""
import json
import platform
import subprocess
import tempfile
from datetime import datetime,timezone
from pathlib import Path
from cli import NativeApp
from support import BIN,EXE,ROOT,RPC,eventually
with tempfile.TemporaryDirectory() as td:
    source=Path(td)/'browser.log'
    source.write_text(''.join(f'temperature={n}\n' for n in range(1,1201)),encoding='utf-8')
    with NativeApp('--input-file',f'source={source}','--set','source.from_start=true','--output-webui','web','--webui-archive',f'web={td}/archive') as app:
        state=json.loads(app.state.read_text())
        with RPC(state['address'],'__manager__',state['token']) as manager:
            url=manager.call('core.call',op='control',args={'target':'web','method':'url','args':{}})['url']
        artifact=ROOT/'quality/artifacts/browser'/f'{platform.system().lower()}-{datetime.now(timezone.utc):%Y%m%dT%H%M%S}'
        subprocess.run(['node',str(ROOT/'quality/tests/browser/workbench.mjs'),url,str(app.state),str(BIN/('log-print'+EXE)),str(artifact)],check=True,timeout=180,cwd=ROOT)
