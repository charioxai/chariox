"""MP-08 / MP-10 / MP-11: Harbor adapter; Chariox remains the agent.

Requires a reviewed, installed Linux runtime bundle and a profile materialized
by the normal Chariox product path in the task environment. This adapter never
reads, uploads or copies a credential. No provider SDK is imported.
"""
import json
import shlex
import tempfile
from pathlib import Path
from uuid import uuid4

from signal_guard import install
install()

from harbor.agents.base import BaseAgent
from harbor.agents.options import AgentOptions
from pydantic import Field
from typing import Literal


class CharioxOptions(AgentOptions):
    runtime_root: str = '/opt/chariox'
    runtime_bundle: str | None = None
    profile_path: str | None = None
    relay_binary: str | None = None
    timeout_seconds: int = Field(default=1800, gt=0)
    placement: Literal['local', 'leased'] = 'local'
    home_kernel_url: str | None = None
    home_session_ref: str | None = None
    home_agent_id: str | None = None
    worker_kernel_id: str | None = None
    home_auth_file: str | None = None
    source_commit: str = Field(pattern=r'^[0-9a-f]{40}$')
    kernel_sha256: str = Field(pattern=r'^[0-9a-f]{64}$')
    local_protocol: int = Field(gt=0)
    provider: Literal['codex', 'claude', 'opencode'] = 'codex'


class CharioxAgent(BaseAgent):
    options_model = CharioxOptions

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        if not self.model_name:
            raise ValueError('MP-08 / MP-10: exact official provider model required')
        for field in ['runtime_root', 'profile_path', 'source_commit', 'kernel_sha256', 'local_protocol', 'provider', 'runtime_bundle', 'placement', 'home_kernel_url', 'home_session_ref', 'home_agent_id', 'worker_kernel_id', 'home_auth_file', 'relay_binary', 'timeout_seconds']:
            setattr(self, field, getattr(self.options, field))
        from contract import admit_placement
        admit_placement(self.options.model_dump())
        self._runner_root = None
        self._workspace = None

    @staticmethod
    def name():
        return 'chariox'

    def version(self):
        return self.source_commit

    async def setup(self, environment):
        self._runner_root = str(self.environment_logs_dir / f'chariox-evals-{uuid4().hex}')
        if self.runtime_bundle:
            from turn import preflight
            bundle = preflight(self.runtime_bundle, self.source_commit, self.kernel_sha256, self.local_protocol)
            await environment.upload_dir(source_dir=bundle, target_dir=self.runtime_root)
        result = await environment.exec(command=f'mkdir -m 700 {shlex.quote(self._runner_root)}')
        if result.return_code != 0:
            raise RuntimeError('MP-11: task runner directory creation failed')
        cwd = await environment.exec(command='pwd')
        if cwd.return_code != 0 or not cwd.stdout.strip().startswith('/'):
            raise RuntimeError('MP-08 / MP-10: task workspace unavailable')
        self._workspace = cwd.stdout.strip()
        for name in ['turn.py', 'contract.py', 'product_status.mjs', 'usage_status.mjs', 'terminal_screen.py', 'relay_target.mjs', 'proxy_prices.py', 'proxy-prices-2026-10-06.json']:
            await environment.upload_file(source_path=Path(__file__).with_name(name),
                                          target_path=f'{self._runner_root}/{name}')
        # Runner-only dependencies live outside the task workspace. Official
        # task files, test instructions and verifier remain untouched.
        venv_root = '/tmp/chariox-evals-python-' + uuid4().hex
        install = await environment.exec(command=shlex.join(['sh', '-c',
            'python3 -m venv ' + shlex.quote(venv_root) + ' 2>/dev/null || '
            '(apt-get update -qq && apt-get install -y -qq python3 python3-venv fonts-dejavu-core && '
            'python3 -m venv ' + shlex.quote(venv_root) + '); ' +
            shlex.quote(venv_root + '/bin/pip') + ' install pyte==0.8.2 pillow==11.3.0']))
        if install.return_code != 0:
            raise RuntimeError('MP-08 / MP-10: task runner dependencies unavailable')
        self._python = venv_root + '/bin/python'
        command = [self._python, f'{self._runner_root}/turn.py', '--preflight',
                   '--runtime-root', self.runtime_root, '--source-commit', self.source_commit,
                   '--kernel-sha256', self.kernel_sha256, '--local-protocol', str(self.local_protocol)]
        result = await environment.exec(command=shlex.join(command))
        (self.logs_dir / 'runtime-preflight.log').write_text((result.stdout or '') + (result.stderr or ''))
        if result.return_code != 0:
            raise RuntimeError('MP-08 / MP-10: task runtime provenance preflight failed; no provider launched')

    async def run(self, instruction, environment, context):
        if not self._runner_root:
            raise RuntimeError('MP-08 / MP-10: setup required')
        input_path = f'{self._runner_root}/input.json'
        result_path = f'{self._runner_root}/result.json'
        request = {'instruction': instruction, 'provider': self.provider,
                   'model': self.model_name, 'profile_path': self.profile_path,
                   'workspace': self._workspace, 'runtime_root': self.runtime_root,
                   'source_commit': self.source_commit, 'kernel_sha256': self.kernel_sha256,
                   'local_protocol': int(self.local_protocol), 'placement': self.placement,
                   'relay_binary': self.relay_binary, 'timeout_seconds': self.timeout_seconds, 'numeric_accounting_required': True,
                   **{key: getattr(self, key) for key in ['home_kernel_url', 'home_session_ref', 'home_agent_id', 'worker_kernel_id', 'home_auth_file']}}
        # Local temporary files contain task instructions only, never profiles.
        with tempfile.TemporaryDirectory(prefix='chariox-evals-input-') as scratch:
            source = Path(scratch) / 'input.json'
            source.write_text(json.dumps(request))
            await environment.upload_file(source_path=source, target_path=input_path)
            result = await environment.exec(command=shlex.join([
                self._python, f'{self._runner_root}/turn.py', '--input', input_path,
                '--result', result_path]))
            local_result = Path(scratch) / 'result.json'
            await environment.download_file(source_path=result_path, target_path=local_result)
            measurement = json.loads(local_result.read_text())
        from proxy_prices import proxy_quote
        if measurement.get('usage'):
            measurement['proxy_cost'] = proxy_quote(self.model_name, measurement['usage'])
        context.metadata = {'chariox': measurement, 'mp_items': ['MP-08', 'MP-10', 'MP-11']}
        # Preserve unavailable counts as null. Do not invent a zero-cost baseline.
        usage = measurement.get('usage')
        if usage is not None:
            from contract import admit_token_usage
            usage = admit_token_usage(usage, measurement['session_id'])
            context.n_input_tokens = usage['input_tokens']
            context.n_cache_tokens = usage['cached_input_tokens']
            context.n_output_tokens = usage['output_tokens']
            if usage['api_equivalent_nanodollars'] is not None:
                context.cost_usd = usage['api_equivalent_nanodollars'] / 1_000_000_000
        if result.return_code != 0 or measurement['status'] != 'completed':
            raise RuntimeError('MP-08 / MP-10: Chariox task did not settle; retain failed task in campaign ledger')
