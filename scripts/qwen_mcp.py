"""Blocking Qwen MCP tool and one-shot Codex Stop hook. No model-side polling."""
import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

import delegate_qwen as delegate

ROOT = Path(__file__).resolve().parents[1]
WORK_TIMEOUT = 570  # Leave time for cleanup and the reply before Codex's 600s deadline.


def report(task):
    result = task.get('result', {})
    return {
        'id': task.get('id'), 'status': task.get('status'),
        'cargo_check': result.get('cargo_check', 'not_run'),
        'summary': str(result.get('summary', ''))[:1000],
        'error': str(result.get('error', ''))[:1000],
        'changed_files': result.get('changed_files', [])[:8],
        'findings': result.get('findings', [])[:5],
        'log_dir': result.get('log_dir'),
    }


def terminate_tree(process):
    if os.name == 'nt':
        subprocess.run(['taskkill', '/PID', str(process.pid), '/T', '/F'],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False, timeout=10)
    else:
        os.killpg(process.pid, signal.SIGKILL)


def run_task(root):
    root = root.resolve()
    board = root / '.memory/01_BLACKBOARD.md'
    _, task = delegate.load_board(board)
    if task.get('status') not in ('queued', 'PENDING_QWEN'):
        return report(task)
    lock = root / 'target/qwen-delegate.lock'
    if lock.exists():
        return {'status': 'busy', 'summary': 'Existing delegation lock; no worker started or restarted.'}
    log = root / 'target/qwen-mcp' / f'worker-{time.time_ns()}.log'
    log.parent.mkdir(parents=True, exist_ok=True)
    command = [sys.executable, str(ROOT / 'scripts/delegate_qwen.py'), '--root', str(root),
               '--timeout', '150', '--check-timeout', '120']
    with log.open('wb') as output:
        process = subprocess.Popen(command, cwd=root, stdout=output, stderr=subprocess.STDOUT,
                                   start_new_session=os.name != 'nt')
        failure = None
        try:
            process.wait(timeout=WORK_TIMEOUT)
        except (subprocess.TimeoutExpired, KeyboardInterrupt):
            terminate_tree(process)
            process.wait(timeout=10)
            # Only our child's lock authorizes cleanup; never steal another worker's lock.
            if lock.exists() and lock.read_text().strip() == str(process.pid):
                lock.unlink()
            failure = 'Worker timeout/interruption; inspect partial diff and logs before retrying.'
    text, current = delegate.load_board(board)
    if process.returncode and failure is None:
        failure = f'Worker exited {process.returncode}; inspect worker log.'
    if failure and current.get('id') == task.get('id') and current.get('status') in ('queued', 'PENDING_QWEN', 'running') and not lock.exists():
        current['status'] = 'blocked'
        current.setdefault('result', {})['error'] = failure
        delegate.save_board(board, text, current)
    result = report(current)
    result['worker_log'] = str(log)
    if current.get('id') != task.get('id'):
        result['error'] = 'Blackboard task changed during execution; inspect worker log.'
    elif failure:
        result['error'] = failure
    return result


def stop_hook(root, event, stderr):
    if event.get('stop_hook_active') is True:
        return 0
    board = root / '.memory/01_BLACKBOARD.md'
    if not board.exists() or delegate.load_board(board)[1].get('status') != 'PENDING_QWEN':
        return 0
    result = run_task(root)
    # A busy worker already owns delivery; do not create a model-side polling loop.
    if result.get('status') == 'busy':
        return 0
    print('Local Qwen finished. Review the result and actual diff; do not resume a paused goal. '
          + json.dumps(result, ensure_ascii=False), file=stderr)
    return 2


def serve():
    # The installed SDK is mcp 2.x; keep hook execution independent of this dependency.
    from mcp.server.mcpserver import MCPServer

    server = MCPServer('bimcraft-local-qwen', log_level='WARNING')

    @server.tool()
    def delegate_qwen() -> dict:
        """Run the queued/PENDING_QWEN blackboard task through local edits and cargo check; return final report only."""
        try:
            return run_task(ROOT)
        except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
            return {'status': 'blocked', 'error': str(error)[:1000]}

    server.run(transport='stdio')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--stop-hook', action='store_true')
    args = parser.parse_args()
    if not args.stop_hook:
        serve()
        return 0
    try:
        raw = sys.stdin.buffer.read(64_001)
        if len(raw) > 64_000:
            raise ValueError('Hook input exceeds 64 KB')
        event = json.loads(raw or b'{}')
        if not isinstance(event, dict):
            raise ValueError('Hook input must be an object')
        code = stop_hook(ROOT, event, sys.stderr)
        if code == 0:
            print('{}')
        return code
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(json.dumps({'systemMessage': f'Qwen hook skipped: {str(error)[:500]}'}))
        return 0


if __name__ == '__main__':
    sys.exit(main())
