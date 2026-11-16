import io
import queue
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest.mock import Mock, patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import qwen_mcp as bridge
import delegate_qwen as delegate


class BridgeTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        (self.root / '.memory').mkdir()
        self.board = self.root / '.memory/01_BLACKBOARD.md'
        self.task = {'id': 'example', 'status': 'PENDING_QWEN', 'result': {}}
        self.write_task()

    def write_task(self):
        self.board.write_text('```qwen-task\n' + json.dumps(self.task) + '\n```\n', encoding='utf-8')

    def test_idle_board_does_not_start_worker(self):
        self.task['status'] = 'needs_review'
        self.write_task()
        with patch.object(bridge.subprocess, 'Popen') as spawn:
            result = bridge.run_task(self.root)
        spawn.assert_not_called()
        self.assertEqual(result['status'], 'needs_review')

    def test_worker_blocks_once_and_returns_only_final_report(self):
        worker = Mock(pid=123, returncode=0)
        def completed(timeout):
            self.assertEqual(timeout, bridge.WORK_TIMEOUT)
            self.task.update(status='needs_review', result={'cargo_check': 'passed', 'summary': 'Done', 'changed_files': ['src/a.rs']})
            self.write_task()
            return 0
        worker.wait.side_effect = completed
        with patch.object(bridge.subprocess, 'Popen', return_value=worker) as spawn:
            report = bridge.run_task(self.root)
        worker.wait.assert_called_once()
        self.assertEqual(report['cargo_check'], 'passed')
        self.assertEqual(report['changed_files'], ['src/a.rs'])
        self.assertIsNotNone(spawn.call_args.kwargs['stdout'])

    def test_timeout_kills_worker_and_marks_its_task_without_requeueing(self):
        worker = Mock(pid=123, returncode=-1)
        worker.wait.side_effect = [subprocess.TimeoutExpired('worker', bridge.WORK_TIMEOUT), -1]
        def started(*args, **kwargs):
            (self.root / 'target/qwen-delegate.lock').write_text('123', encoding='utf-8')
            self.task['status'] = 'running'
            self.write_task()
            return worker
        with patch.object(bridge.subprocess, 'Popen', side_effect=started), patch.object(bridge, 'terminate_tree') as terminate:
            report = bridge.run_task(self.root)
        terminate.assert_called_once_with(worker)
        self.assertEqual(report['status'], 'blocked')
        self.assertIn('timeout', report['error'].lower())
        self.assertFalse((self.root / 'target/qwen-delegate.lock').exists())
        self.assertEqual(delegate.load_board(self.board)[1]['status'], 'blocked')

    def test_existing_worker_is_not_restarted_or_its_board_changed(self):
        (self.root / 'target').mkdir()
        (self.root / 'target/qwen-delegate.lock').write_text('456', encoding='utf-8')
        original = self.board.read_bytes()
        with patch.object(bridge.subprocess, 'Popen') as spawn:
            self.assertEqual(bridge.run_task(self.root)['status'], 'busy')
        spawn.assert_not_called()
        self.assertEqual(self.board.read_bytes(), original)

    def test_failed_worker_does_not_leave_pending_task_for_repeated_stop_runs(self):
        worker = Mock(pid=123, returncode=2)
        with patch.object(bridge.subprocess, 'Popen', return_value=worker):
            result = bridge.run_task(self.root)
        self.assertEqual(result['status'], 'blocked')
        self.assertEqual(delegate.load_board(self.board)[1]['status'], 'blocked')
        with patch.object(bridge, 'run_task') as run:
            self.assertEqual(bridge.stop_hook(self.root, {}, io.StringIO()), 0)
        run.assert_not_called()

    def test_stop_pending_wakes_codex_once_with_finished_report(self):
        stderr = io.StringIO()
        with patch.object(bridge, 'run_task', return_value={'status': 'needs_review', 'cargo_check': 'passed'}) as run:
            self.assertEqual(bridge.stop_hook(self.root, {}, stderr), 2)
        run.assert_called_once_with(self.root)
        self.assertIn('passed', stderr.getvalue())

    def test_stop_ignores_recursive_or_non_pending_turns(self):
        with patch.object(bridge, 'run_task') as run:
            self.assertEqual(bridge.stop_hook(self.root, {'stop_hook_active': True}, io.StringIO()), 0)
            self.task['status'] = 'queued'
            self.write_task()
            self.assertEqual(bridge.stop_hook(self.root, {}, io.StringIO()), 0)
        run.assert_not_called()

    def test_stdio_handshake_tool_discovery_and_final_result(self):
        self.task['status'] = 'needs_review'
        self.task['result'] = {'cargo_check': 'passed', 'summary': 'Ready for review'}
        self.write_task()
        script = ('import sys; from pathlib import Path; '
                  'sys.path.insert(0,sys.argv[1]); import qwen_mcp; '
                  'qwen_mcp.ROOT=Path(sys.argv[2]); qwen_mcp.serve()')
        with tempfile.TemporaryFile() as errors:
            process = subprocess.Popen([sys.executable, '-c', script,
                str(Path(bridge.__file__).parent), str(self.root)],
                stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=errors)
            responses = queue.Queue()
            def read_responses():
                for line in process.stdout:
                    responses.put(json.loads(line))
            reader = threading.Thread(target=read_responses, daemon=True)
            reader.start()
            def send(method, params, request_id=None):
                message = {'jsonrpc': '2.0', 'method': method, 'params': params}
                if request_id is not None:
                    message['id'] = request_id
                process.stdin.write(json.dumps(message).encode() + b'\n')
                process.stdin.flush()
                if request_id is not None:
                    response = responses.get(timeout=10)
                    self.assertEqual(response['id'], request_id)
                    self.assertNotIn('error', response)
                    return response['result']
            try:
                send('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {},
                    'clientInfo': {'name': 'test', 'version': '1'}}, 1)
                send('notifications/initialized', {})
                tools = send('tools/list', {}, 2)
                self.assertEqual([tool['name'] for tool in tools['tools']], ['delegate_qwen'])
                result = send('tools/call', {'name': 'delegate_qwen', 'arguments': {}}, 3)
                self.assertFalse(result.get('isError'))
                payload = json.loads(next(item['text'] for item in result['content'] if item['type'] == 'text'))
                self.assertEqual(payload['status'], 'needs_review')
                self.assertEqual(payload['cargo_check'], 'passed')
            except queue.Empty:
                errors.seek(0)
                self.fail('MCP protocol timeout: ' + errors.read(2000).decode(errors='replace'))
            finally:
                process.kill()
                process.wait(timeout=5)
                reader.join(timeout=2)
                process.stdin.close()
                process.stdout.close()


if __name__ == '__main__':
    unittest.main()
