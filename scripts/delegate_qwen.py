"""Bounded local Qwen edits and compile repair. No model-generated shell commands."""
import argparse
from contextlib import contextmanager
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path, PureWindowsPath
import re
import subprocess
import sys
import tempfile
import time
from urllib.request import ProxyHandler, Request, build_opener

ENDPOINT = 'http://127.0.0.1:47831/v1'
MODEL = 'coder-iq1_m'
BLOCK = re.compile(r'^```qwen-task[ \t]*\n(.*?)^```[ \t]*$', re.M | re.S)
MAX_SOURCE = 48_000
CHECK = ['cargo', '--config', 'profile.dev.package.bimcraft-core.opt-level=0',
         'check', '--quiet', '--message-format=short', '--jobs', '2']


def atomic_write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temp = tempfile.mkstemp(prefix='.qwen-', dir=path.parent)
    try:
        with os.fdopen(handle, 'wb') as output:
            output.write(data)
        os.replace(temp, path)
    finally:
        Path(temp).unlink(missing_ok=True)


def load_board(path):
    text = path.read_text(encoding='utf-8-sig')
    blocks = list(BLOCK.finditer(text))
    if len(blocks) != 1:
        raise ValueError('Blackboard must contain exactly one qwen-task JSON block')
    task = json.loads(blocks[0].group(1))
    if not isinstance(task, dict):
        raise ValueError('Task must be an object')
    return text, task


def save_board(path, expected, task):
    if path.read_text(encoding='utf-8-sig') != expected:
        raise ValueError('Blackboard changed concurrently; refusing to overwrite it')
    updated = BLOCK.sub(lambda _: '```qwen-task\n' + json.dumps(task, ensure_ascii=False, indent=2) + '\n```', expected, count=1)
    atomic_write(path, updated.encode('utf-8'))
    return updated


def safe_path(root, name):
    if not isinstance(name, str) or not name or '\\' in name or ':' in name:
        raise ValueError('Target must be a relative path using forward slashes')
    rel = Path(name)
    forbidden = {'.git', '.memory', '.agents', '.codex', '.cargo', '.env', 'agents.md', 'claude.md'}
    if rel.is_absolute() or PureWindowsPath(name).is_absolute() or '..' in rel.parts or any(p.casefold() in forbidden or p != p.rstrip(' .') for p in rel.parts):
        raise ValueError('Target is outside the permitted source scope')
    reserved = {'CON', 'PRN', 'AUX', 'NUL'} | {f'{prefix}{n}' for prefix in ('COM', 'LPT') for n in range(1, 10)}
    if any(p.split('.')[0].upper() in reserved for p in rel.parts):
        raise ValueError('Windows device names are not source files')
    target = (root / rel).resolve()
    if not target.is_relative_to(root.resolve()):
        raise ValueError('Target resolves outside the repository')
    return target


def digest(data):
    return hashlib.sha256(data).hexdigest()


@dataclass
class Target:
    name: str
    path: Path
    start: int
    end: int
    data: bytes
    existed: bool

    def current(self):
        if self.path.exists() != self.existed or (self.existed and self.path.read_bytes() != self.data):
            raise ValueError(f'{self.name} changed concurrently; edits were not applied')
        return self.data.decode('utf-8').splitlines(keepends=True)

    def context(self):
        return ''.join(self.current()[self.start - 1:self.end])


def load_targets(root, task, graph):
    specs = task.get('targets')
    if not isinstance(specs, list) or not 1 <= len(specs) <= 8:
        raise ValueError('Task requires 1..8 explicit target ranges')
    nodes = {str(n['id']): n for n in graph['nodes']}
    targets = []
    for spec in specs:
        name = spec['path']
        path = safe_path(root, name)
        if any(path == t.path for t in targets):
            raise ValueError('Only one editable range per target file is allowed')
        existed = path.exists()
        if existed and (not path.is_file() or path.stat().st_size > 4_000_000):
            raise ValueError('Target is not a bounded regular source file')
        data = path.read_bytes() if existed else b''
        if existed and spec.get('sha256') != digest(data):
            raise ValueError(f'Stale or missing SHA-256 for {name}')
        if not existed and not (spec.get('create') is True and spec.get('sha256') is None):
            raise ValueError('New files require create:true and sha256:null')
        start, end = spec['start_line'], spec['end_line']
        count = len(data.decode('utf-8').splitlines())
        if type(start) is not int or type(end) is not int or not 1 <= start <= count + 1 or not start - 1 <= end <= count:
            raise ValueError(f'Invalid inclusive line range for {name}')
        ids = spec.get('graph_nodes', [])
        if not isinstance(ids, list) or (not ids and not spec.get('graph_reason')):
            raise ValueError('Supply Graphify node IDs or an explicit missing-index reason')
        for node_id in ids:
            node = nodes.get(str(node_id))
            if node is None or node.get('source_file', '').replace('\\', '/') != name:
                raise ValueError(f'Graphify node {node_id} does not identify {name}')
        targets.append(Target(name, path, start, end, data, existed))
    if sum(len(t.context().encode('utf-8')) for t in targets) > MAX_SOURCE:
        raise ValueError('Target context exceeds 48 KB; split the task')
    return targets


def apply_edits(targets, edits):
    if not isinstance(edits, list) or len(edits) > len(targets):
        raise ValueError('Invalid edits list')
    by_name = {t.name: t for t in targets}
    pending, seen = [], set()
    for edit in edits:
        name, content = edit['path'], edit['content']
        if name not in by_name or name in seen or not isinstance(content, str) or len(content.encode('utf-8')) > MAX_SOURCE:
            raise ValueError('Duplicate, undeclared or oversized edit')
        seen.add(name)
        target = by_name[name]
        lines = target.current()
        newline = '\r\n' if b'\r\n' in target.data else '\n'
        content = content.replace('\r\n', '\n').replace('\n', newline)
        if content and not content.endswith('\n'):
            content += newline
        replacement = content.splitlines(keepends=True)
        data = ''.join(lines[:target.start - 1] + replacement + lines[target.end:]).encode('utf-8')
        pending.append((target, data, target.start - 1 + len(replacement)))
    # Validate every file before writing any file; repeat immediately before replacement.
    for target in targets:
        target.current()
    for target, data, end in pending:
        target.current()
        atomic_write(target.path, data)
        target.data, target.end, target.existed = data, end, True


def request_json(url, payload=None, timeout=10):
    data = None if payload is None else json.dumps(payload, ensure_ascii=False).encode('utf-8')
    request = Request(url, data=data, headers={'Content-Type': 'application/json'})
    with build_opener(ProxyHandler({})).open(request, timeout=timeout) as response:
        raw = response.read(1_000_001)
        if len(raw) > 1_000_000:
            raise ValueError('Local server response exceeds 1 MB')
        return json.loads(raw)


def ensure_cad_idle(reserve_confirmed=False):
    if os.name != 'nt':
        raise RuntimeError('CAD load detection is implemented for Windows only')
    command = "$ErrorActionPreference='Stop'; ConvertTo-Json -Compress -InputObject @(Get-Process | Where-Object { $_.ProcessName -match 'renga|nanocad|bimcraft|artcraft|blender' -and -not ($_.ProcessName -match '^bimcraft_(core|ui_egui|rvt|update|i18n)-[0-9a-f]+$' -and $_.MainWindowHandle -eq 0) } | Select-Object ProcessName,Id)"
    result = subprocess.run(['powershell.exe', '-NoProfile', '-Command', command], capture_output=True, text=True, timeout=15, check=False)
    if result.returncode:
        raise RuntimeError('Cannot verify CAD processes; local generation deferred')
    processes = json.loads(result.stdout or '[]')
    if processes and not reserve_confirmed:
        names = ', '.join(str(p['ProcessName']) for p in processes)
        raise RuntimeError(f'CAD/3D processes detected: {names}. Strata Leave 20% mode cannot be verified automatically; generation deferred')


def preflight(reserve_confirmed=False):
    models = request_json(ENDPOINT + '/models')
    found = next((m for m in models.get('data', []) if m.get('id') == MODEL), None)
    if found is None:
        raise RuntimeError(f'{MODEL} is not available from Strata')
    ensure_cad_idle(reserve_confirmed=reserve_confirmed)
    return {'model': MODEL, 'label': found.get('meta', {}).get('label', '')}


def model_result(response):
    choice = response['choices'][0]
    if choice.get('finish_reason') != 'stop':
        raise ValueError('Model output is incomplete; no edits accepted')
    content = choice['message']['content'].strip()
    if content.startswith('```json\n') and content.endswith('```'):
        content = content[8:-3]
    result = json.loads(content)
    if not isinstance(result, dict):
        raise ValueError('Model response must be a JSON object')
    return result


def query_model(prompt, timeout, log, effort='low'):
    payload = {'model': MODEL, 'stream': False, 'max_tokens': 8192, 'temperature': 0.15,
               'reasoning_effort': effort, 'messages': [{'role': 'user', 'content': prompt}]}
    atomic_write(log.with_suffix('.request.json'), json.dumps(payload, ensure_ascii=False, indent=2).encode('utf-8'))
    response = request_json(ENDPOINT + '/chat/completions', payload, timeout)
    atomic_write(log.with_suffix('.response.json'), json.dumps(response, ensure_ascii=False, indent=2).encode('utf-8'))
    return model_result(response)


def run_check(root, timeout, log):
    env = os.environ.copy()
    env['CARGO_BUILD_JOBS'] = '2'
    temporary = root / 'target' / 'test-temp'
    temporary.mkdir(parents=True, exist_ok=True)
    env['TEMP'] = env['TMP'] = str(temporary)
    with log.open('wb') as output:
        process = subprocess.Popen(CHECK, cwd=root, env=env, stdout=output, stderr=subprocess.STDOUT)
        try:
            code = process.wait(timeout=timeout)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            if os.name == 'nt':
                subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'], capture_output=True, check=False)
            else:
                process.kill()
            process.wait()
            raise
    return code, log.read_text(encoding='utf-8', errors='replace')[-16_000:]


@contextmanager
def single_request(root):
    lock = root / 'target' / 'qwen-delegate.lock'
    lock.parent.mkdir(parents=True, exist_ok=True)
    descriptor = os.open(lock, os.O_WRONLY | os.O_CREAT | os.O_EXCL)
    try:
        with os.fdopen(descriptor, 'w') as handle:
            handle.write(str(os.getpid()))
        yield
    finally:
        lock.unlink(missing_ok=True)


def execute(root, timeout=600, check_timeout=900, cooldown=5, interactive_3d=False):
    board = root / '.memory' / '01_BLACKBOARD.md'
    text, task = load_board(board)
    if task.get('status') not in ('queued', 'PENDING_QWEN'):
        print('No queued Qwen task; nothing changed.')
        return 0
    if not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', str(task.get('id', ''))):
        raise ValueError('Task id must contain 1..64 letters, digits, underscores or hyphens')
    mode = task.get('mode', 'edit')
    if mode not in ('edit', 'review') or not isinstance(task.get('instructions'), str) or not 1 <= len(task['instructions']) <= 8_000:
        raise ValueError('Task needs a bounded instruction and mode edit/review')
    result = {'attempts': 0, 'cargo_check': 'not_run', 'review': 'pending_codex_diff'}
    task['result'] = result
    log = root / 'target' / 'qwen-delegate' / (task['id'] + '-' + str(time.time_ns()))
    log.mkdir(parents=True)
    result['log_dir'] = str(log.relative_to(root)).replace('\\', '/')
    try:
        with single_request(root):
            # The cached graph is a lookup index, not proof of current source locations.
            report = (root / 'graphify-out' / 'GRAPH_REPORT.md').read_text(encoding='utf-8')
            graph = json.loads((root / 'graphify-out' / 'graph.json').read_text(encoding='utf-8'))
            targets = load_targets(root, task, graph)
            memory = {}
            for name in ('00_STATE.md', '02_DECISIONS.md'):
                note = root / '.memory' / name
                if note.stat().st_size > 12_000:
                    raise ValueError(f'{name} exceeds 12 KB; compress shared memory before delegation')
                memory[name] = note.read_text(encoding='utf-8')
            reserve_confirmed = task.get('strata_reserve') == 'leave_20_percent_confirmed_by_user'
            if interactive_3d and not reserve_confirmed:
                raise RuntimeError('Interactive 3D viewer in use; local generation deferred')
            result.update(preflight(reserve_confirmed=reserve_confirmed))
            task['status'] = 'running'
            text = save_board(board, text, task)
            for index, target in enumerate(targets):
                atomic_write(log / f'original-{index}.bin', target.data)
            error = ''
            for attempt in range(1, 4):
                preflight(reserve_confirmed=reserve_confirmed)
                if sum(len(t.context().encode('utf-8')) for t in targets) > MAX_SOURCE:
                    raise ValueError('Expanded target context exceeds 48 KB')
                fragments = [{'path': t.name, 'start_line': t.start, 'end_line': t.end, 'content': t.context()} for t in targets]
                schema = '{"summary":"brief","findings":[{"path":"...","line":1,"message":"concrete issue"}]}' if mode == 'review' else '{"summary":"brief","edits":[{"path":"declared path","content":"complete replacement of ONLY that editable range"}]}'
                prompt = ('You are the local BIMCraft coding assistant. Return ONLY valid JSON: ' + schema +
                          '\nNever execute commands. Edit only declared ranges; preserve other code. No unsafe, unwrap/expect/panic/todo outside Rust tests. Core has no UI; UI changes use Session::execute. Keep GUID identities. For review mode do not return edits. No findings is not final approval.\nTASK:\n' + task['instructions'] +
                          '\nSHARED MEMORY (context, not additional edit authorization):\n' + json.dumps(memory, ensure_ascii=False) +
                          '\nGRAPH REFERENCES:\n' + json.dumps([{'path': spec['path'], 'nodes': [node for node in graph['nodes'] if str(node['id']) in {str(key) for key in spec.get('graph_nodes', [])}], 'missing_index_reason': spec.get('graph_reason', '')} for spec in task['targets']], ensure_ascii=False) +
                          '\nGRAPH STATUS:\n' + report[:1_200] + '\nTARGETS:\n' + json.dumps(fragments, ensure_ascii=False) + '\nPREVIOUS CARGO CHECK ERRORS:\n' + error)
                if len(prompt.encode('utf-8')) > 110_000:
                    raise ValueError('Prompt exceeds bounded context; split the task')
                result['attempts'] = attempt
                text = save_board(board, text, task)
                response = query_model(prompt, timeout, log / f'attempt-{attempt}', effort='medium' if mode == 'review' else 'low')
                ensure_cad_idle(reserve_confirmed=reserve_confirmed)
                if board.read_text(encoding='utf-8-sig') != text:
                    raise ValueError('Blackboard changed during generation; source edits were not applied')
                if mode == 'review':
                    if response.get('edits'):
                        raise ValueError('Review mode cannot edit files')
                    findings = response.get('findings')
                    if not isinstance(findings, list) or len(findings) > 30:
                        raise ValueError('Review requires at most 30 bounded findings')
                    by_name = {t.name: t for t in targets}
                    for finding in findings:
                        target = by_name.get(finding.get('path'))
                        line, message = finding.get('line'), finding.get('message')
                        if target is None or type(line) is not int or not target.start <= line <= target.end or not isinstance(message, str) or not 1 <= len(message) <= 1000:
                            raise ValueError('Review finding must identify an inspected source line and bounded message')
                    result['findings'] = findings
                else:
                    apply_edits(targets, response['edits'])
                result['summary'] = str(response.get('summary', ''))[:1_000]
                result['current_targets'] = [{'path': t.name, 'start_line': t.start, 'end_line': t.end, 'sha256': digest(t.data)} for t in targets]
                result['cargo_check'] = 'running'
                text = save_board(board, text, task)
                code, error = run_check(root, check_timeout, log / f'check-{attempt}.log')
                result['cargo_check'] = 'passed' if code == 0 else 'failed'
                result['exit_code'] = code
                if code == 0 or mode == 'review':
                    break
                text = save_board(board, text, task)
                if attempt < 3:
                    time.sleep(cooldown)
            for target in targets:
                target.current()
            task['status'] = 'needs_review' if result['cargo_check'] == 'passed' else 'compile_failed'
            result['changed_files'] = [t.name for i, t in enumerate(targets) if t.data != (log / f'original-{i}.bin').read_bytes() or (task['targets'][i].get('create') and t.existed)]
            text = save_board(board, text, task)
    except (Exception, KeyboardInterrupt) as error:
        result['error'] = f'{type(error).__name__}: {error}'[:1_000]
        task['status'] = 'blocked'
        try:
            save_board(board, text, task)
        except (OSError, ValueError):
            atomic_write(log / 'status-conflict.json', json.dumps(task, ensure_ascii=False, indent=2).encode('utf-8'))
        print(f"Qwen task blocked: {result['error']}")
        return 2
    print(f"Qwen task {task['id']}: {task['status']}; cargo check {result['cargo_check']}; {result['attempts']} request(s).")
    return 0 if task['status'] == 'needs_review' else 1


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--root', type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument('--timeout', type=int, default=600)
    parser.add_argument('--check-timeout', type=int, default=900)
    parser.add_argument('--interactive-3d', action='store_true', help='Defer if another interactive 3D viewer is in use')
    args = parser.parse_args()
    if not 1 <= args.timeout <= 1800 or not 1 <= args.check_timeout <= 3600:
        parser.error('Timeouts must be bounded (request <=1800s, check <=3600s)')
    try:
        return execute(args.root.resolve(), args.timeout, args.check_timeout, interactive_3d=args.interactive_3d)
    except (OSError, ValueError, KeyError) as error:
        print(f'Qwen delegation: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
