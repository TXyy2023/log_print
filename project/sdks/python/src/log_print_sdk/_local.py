"""Optional owned local Core. Never touches another instance or downloads tools."""
from __future__ import annotations

import asyncio
import json
import os
from pathlib import Path
import secrets
import shutil
import tempfile

from ._models import ConfigurationError


class LocalCore:
    def __init__(self, binary, options):
        self.binary = binary
        self.options = options or {}
        self.temp = None
        self.process = None

    async def start(self):
        if self.options.get('transport', 'tcp') != 'tcp':
            raise ConfigurationError('Python SDK requires TCP', code='unsupported_transport')
        binary = self.binary or os.environ.get('LOG_PRINT_CORE_BIN') or shutil.which('log-print-core')
        if not binary:
            cli = shutil.which('log-print')
            if cli:
                sibling = Path(cli).with_name('log-print-core.exe' if os.name == 'nt' else 'log-print-core')
                if sibling.is_file():
                    binary = str(sibling)
        if not binary:
            raise ConfigurationError('Install log-print-core on PATH, set LOG_PRINT_CORE_BIN, or pass core_binary=...', code='core_not_found')
        self.temp = tempfile.TemporaryDirectory(prefix='log-print-python-')
        path = Path(self.temp.name)
        self.tokens = {name: secrets.token_urlsafe(32) for name in ('python-input', 'python-output')}
        runtime = dict(config=dict(core=dict(self.options, transport='tcp'), plugins=[
            dict(id='python-input', role='input', bin='unused', streams=[dict(id='python')]),
            dict(id='python-output', role='output', bin='unused', reads=['python']),
        ]), admin_token=secrets.token_urlsafe(32), plugin_tokens=self.tokens)
        config = path / 'runtime.json'
        with config.open('x', encoding='utf-8') as out:
            if os.name != 'nt':
                os.chmod(config, 0o600)
            json.dump(runtime, out)
        ready = path / 'ready.json'
        try:
            self.process = await asyncio.create_subprocess_exec(
                str(binary), '--runtime-config', str(config), '--ready-file', str(ready),
                stdin=asyncio.subprocess.PIPE, stdout=asyncio.subprocess.DEVNULL,
                stderr=asyncio.subprocess.DEVNULL)
            async with asyncio.timeout(10):
                while True:
                    if self.process.returncode is not None:
                        raise ConfigurationError('local Core exited before ready; verify binary and core_options', code='core_start_failed')
                    try:
                        self.address = json.loads(ready.read_text())['address']
                        return self
                    except (OSError, ValueError, KeyError):
                        await asyncio.sleep(.02)
        except OSError as error:
            await self.close()
            raise ConfigurationError('cannot execute local Core; check core_binary path and permissions', code='core_not_found') from error
        except BaseException:
            await self.close()
            raise

    async def close(self):
        if self.process:
            if self.process.stdin:
                self.process.stdin.close()
            try:
                async with asyncio.timeout(3):
                    await self.process.wait()
            except TimeoutError:
                if self.process.returncode is None:
                    self.process.kill()
                await self.process.wait()
            if self.process.stdin:
                try:
                    await self.process.stdin.wait_closed()
                except (OSError, RuntimeError):
                    pass
        if self.temp:
            self.temp.cleanup()
            self.temp = None
