import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import delegate_qwen as qwen


class DelegationTests(unittest.TestCase):
    def test_explicit_user_reserve_is_propagated_to_all_load_checks(self):
        text, task = qwen.load_board(self.board)
        task['strata_reserve'] = 'leave_20_percent_confirmed_by_user'
        qwen.save_board(self.board, text, task)
        with patch.object(qwen, 'preflight', return_value={}) as preflight, \
             patch.object(qwen, 'ensure_cad_idle') as load_check, \
             patch.object(qwen, 'query_model', return_value={'summary': 'done', 'edits': []}), \
             patch.object(qwen, 'run_check', return_value=(0, '')):
            self.assertEqual(qwen.execute(self.root, cooldown=0, interactive_3d=True), 0)
        self.assertEqual(preflight.call_count, 2)
        for call in preflight.call_args_list:
            self.assertEqual(call.kwargs, {'reserve_confirmed': True})
        load_check.assert_called_once_with(reserve_confirmed=True)

    def test_pending_hook_task_uses_existing_edit_and_check_cycle(self):
        text, task = qwen.load_board(self.board)
        task['status'] = 'PENDING_QWEN'
        qwen.save_board(self.board, text, task)
        with patch.object(qwen, 'preflight', return_value={}), patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', return_value={'summary': 'done', 'edits': []}), \
             patch.object(qwen, 'run_check', return_value=(0, '')) as check:
            self.assertEqual(qwen.execute(self.root, cooldown=0), 0)
        check.assert_called_once()
        self.assertEqual(qwen.load_board(self.board)[1]['status'], 'needs_review')

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.memory = self.root / '.memory'
        self.memory.mkdir()
        (self.memory / '00_STATE.md').write_text('Unique shared state: BIM work is paused.', encoding='utf-8')
        (self.memory / '02_DECISIONS.md').write_text('Unique shared decision: immutable GUIDs.', encoding='utf-8')
        (self.root / 'src').mkdir()
        self.source = self.root / 'src/example.rs'
        self.source.write_bytes(b'// keep\r\nfn value() -> u32 { 0 }\r\n// keep too\r\n')
        self.task = {
            'id': 'test-task', 'status': 'queued', 'mode': 'edit',
            'instructions': 'Make value return one.',
            'targets': [{'path': 'src/example.rs', 'start_line': 2, 'end_line': 2,
                         'sha256': hashlib.sha256(self.source.read_bytes()).hexdigest(),
                         'graph_nodes': [], 'graph_reason': 'Not indexed in the cached graph.'}],
        }
        self.board = self.memory / '01_BLACKBOARD.md'
        self.board.write_text('Keep this prose.\n```qwen-task\n' + json.dumps(self.task) + '\n```\n[[00_STATE]]\n', encoding='utf-8')
        graph = self.root / 'graphify-out'
        graph.mkdir()
        (graph / 'GRAPH_REPORT.md').write_text('Cached graph.', encoding='utf-8')
        (graph / 'graph.json').write_text('{"nodes":[],"edges":[]}', encoding='utf-8')

    def targets(self):
        return qwen.load_targets(self.root, self.task, {'nodes': []})

    def test_range_replacement_preserves_other_lines_and_crlf(self):
        targets = self.targets()
        qwen.apply_edits(targets, [{'path': 'src/example.rs', 'content': 'fn value() -> u32 { 1 }'}])
        self.assertEqual(self.source.read_bytes(), b'// keep\r\nfn value() -> u32 { 1 }\r\n// keep too\r\n')

    def test_concurrent_source_edit_is_not_overwritten(self):
        targets = self.targets()
        self.source.write_text('user edit', encoding='utf-8')
        with self.assertRaises(ValueError):
            qwen.apply_edits(targets, [{'path': 'src/example.rs', 'content': 'overwrite'}])
        self.assertEqual(self.source.read_text(), 'user edit')

    def test_model_cannot_edit_undeclared_files_or_repeat_a_file(self):
        for edits in [[{'path': '../outside.rs', 'content': 'bad'}],
                      [{'path': 'src/example.rs', 'content': 'a'}, {'path': 'src/example.rs', 'content': 'b'}]]:
            with self.subTest(edits=edits), self.assertRaises(ValueError):
                qwen.apply_edits(self.targets(), edits)
        self.assertIn(b'fn value() -> u32 { 0 }', self.source.read_bytes())

    def test_stale_hash_and_paths_outside_repository_are_rejected(self):
        for name in ['../outside.rs', '.memory/00_STATE.md', '.git/config', '.GIT/config', 'C:/outside.rs', 'src/NUL.rs', '.git./config']:
            with self.subTest(name=name), self.assertRaises(ValueError):
                qwen.safe_path(self.root, name)
        self.task['targets'][0]['sha256'] = '0' * 64
        with self.assertRaises(ValueError):
            self.targets()

    def test_graph_provenance_must_match_the_target(self):
        self.task['targets'][0]['graph_nodes'] = ['some-node']
        with self.assertRaises(ValueError):
            qwen.load_targets(self.root, self.task, {'nodes': [{'id': 'some-node', 'source_file': 'other.rs'}]})

    def test_blackboard_update_preserves_prose_and_detects_concurrent_edits(self):
        text, task = qwen.load_board(self.board)
        task['status'] = 'running'
        changed = qwen.save_board(self.board, text, task)
        self.assertIn('Keep this prose.', changed)
        self.assertIn('[[00_STATE]]', changed)
        self.assertTrue(changed.endswith('```\n[[00_STATE]]\n'))
        with self.assertRaises(ValueError):
            qwen.save_board(self.board, text, task)

    def test_multiple_active_json_blocks_are_rejected(self):
        self.board.write_text(self.board.read_text() * 2, encoding='utf-8')
        with self.assertRaises(ValueError):
            qwen.load_board(self.board)

    def test_truncated_or_non_json_model_output_is_not_accepted(self):
        with self.assertRaises(ValueError):
            qwen.model_result({'choices': [{'finish_reason': 'length', 'message': {'content': '{}'}}]})
        with self.assertRaises(ValueError):
            qwen.model_result({'choices': [{'finish_reason': 'stop', 'message': {'content': 'not JSON'}}]})

    def test_compile_errors_are_repaired_locally_and_still_require_review(self):
        responses = [{'summary': 'first', 'edits': [{'path': 'src/example.rs', 'content': 'fn value() -> u32 { wrong }'}]},
                     {'summary': 'fixed', 'edits': [{'path': 'src/example.rs', 'content': 'fn value() -> u32 { 1 }'}]}]
        with patch.object(qwen, 'preflight', return_value={}), \
             patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', side_effect=responses) as generate, \
             patch.object(qwen, 'run_check', side_effect=[(1, 'error: wrong not found'), (0, '')]):
            code = qwen.execute(self.root, timeout=1, check_timeout=1, cooldown=0)
        self.assertEqual(code, 0)
        self.assertEqual(generate.call_count, 2)
        self.assertIn('wrong not found', generate.call_args.args[0])
        self.assertIn('Unique shared state', generate.call_args.args[0])
        self.assertIn('Unique shared decision', generate.call_args.args[0])
        _, task = qwen.load_board(self.board)
        self.assertEqual(task['status'], 'needs_review')
        self.assertEqual(task['result']['attempts'], 2)
        self.assertEqual(task['result']['cargo_check'], 'passed')

    def test_failed_check_is_not_accepted_and_retries_are_capped(self):
        with patch.object(qwen, 'preflight', return_value={}), \
             patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', return_value={'summary': 'unchanged', 'edits': []}) as generate, \
             patch.object(qwen, 'run_check', return_value=(1, 'error: still broken')):
            self.assertEqual(qwen.execute(self.root, timeout=1, check_timeout=1, cooldown=0), 1)
        self.assertEqual(generate.call_count, 3)
        self.assertEqual(qwen.load_board(self.board)[1]['status'], 'compile_failed')

    def test_cad_blocks_generation_without_touching_source(self):
        with patch.object(qwen, 'preflight', side_effect=RuntimeError('CAD reserve cannot be confirmed')), \
             patch.object(qwen, 'query_model') as generate:
            self.assertEqual(qwen.execute(self.root), 2)
        generate.assert_not_called()
        self.assertIn(b'{ 0 }', self.source.read_bytes())
        self.assertEqual(qwen.load_board(self.board)[1]['status'], 'blocked')

    def test_explicit_other_3d_viewer_blocks_and_records_the_reason(self):
        with patch.object(qwen, 'preflight') as preflight, patch.object(qwen, 'query_model') as generate:
            self.assertEqual(qwen.execute(self.root, interactive_3d=True), 2)
        preflight.assert_not_called()
        generate.assert_not_called()
        self.assertEqual(qwen.load_board(self.board)[1]['status'], 'blocked')

    def test_review_records_findings_without_editing_or_retrying(self):
        text, task = qwen.load_board(self.board)
        task['mode'] = 'review'
        qwen.save_board(self.board, text, task)
        finding = {'path': 'src/example.rs', 'line': 2, 'message': 'Example finding'}
        with patch.object(qwen, 'preflight', return_value={}), patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', return_value={'summary': 'review', 'findings': [finding]}) as generate, \
             patch.object(qwen, 'run_check', return_value=(0, '')):
            self.assertEqual(qwen.execute(self.root, cooldown=0), 0)
        self.assertEqual(generate.call_count, 1)
        self.assertIn(b'{ 0 }', self.source.read_bytes())
        self.assertEqual(qwen.load_board(self.board)[1]['result']['findings'], [finding])

    def test_review_cannot_invent_unread_source_locations(self):
        text, task = qwen.load_board(self.board)
        task['mode'] = 'review'
        qwen.save_board(self.board, text, task)
        with patch.object(qwen, 'preflight', return_value={}), patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', return_value={'summary': 'bad review', 'findings': [{'path': 'elsewhere.rs', 'line': 1, 'message': 'invented'}]}), \
             patch.object(qwen, 'run_check', return_value=(0, '')):
            self.assertEqual(qwen.execute(self.root, cooldown=0), 2)

    def test_cancelling_blackboard_during_generation_prevents_source_edits(self):
        def cancel(*args, **kwargs):
            self.board.write_text('User cancelled the task.\n', encoding='utf-8')
            return {'summary': 'late answer', 'edits': [{'path': 'src/example.rs', 'content': 'overwrite'}]}
        with patch.object(qwen, 'preflight', return_value={}), patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', side_effect=cancel), patch.object(qwen, 'run_check', return_value=(0, '')):
            self.assertEqual(qwen.execute(self.root, cooldown=0), 2)
        self.assertIn(b'{ 0 }', self.source.read_bytes())
        self.assertEqual(self.board.read_text(), 'User cancelled the task.\n')

    def test_empty_file_range_survives_repeated_replacements(self):
        self.source.write_bytes(b'')
        self.task['targets'][0].update(start_line=1, end_line=0, sha256=hashlib.sha256(b'').hexdigest())
        targets = self.targets()
        qwen.apply_edits(targets, [{'path': 'src/example.rs', 'content': 'first\nsecond'}])
        self.assertEqual(targets[0].context(), 'first\nsecond\n')
        qwen.apply_edits(targets, [{'path': 'src/example.rs', 'content': 'new first\nnew second\nnew third'}])
        self.assertEqual(targets[0].context(), 'new first\nnew second\nnew third\n')

    def test_unchanged_new_target_is_not_reported_as_created(self):
        text, task = qwen.load_board(self.board)
        task['targets'][0].update(path='src/new.rs', start_line=1, end_line=0, sha256=None, create=True)
        qwen.save_board(self.board, text, task)
        with patch.object(qwen, 'preflight', return_value={}), patch.object(qwen, 'ensure_cad_idle'), \
             patch.object(qwen, 'query_model', return_value={'summary': 'no changes', 'edits': []}), \
             patch.object(qwen, 'run_check', return_value=(0, '')):
            self.assertEqual(qwen.execute(self.root, cooldown=0), 0)
        self.assertEqual(qwen.load_board(self.board)[1]['result']['changed_files'], [])


if __name__ == '__main__':
    unittest.main()

class ConfirmedReserveTests(unittest.TestCase):
    def test_open_cad_still_blocks_without_explicit_reserve_confirmation(self):
        result=type('Result',(),{'returncode':0,'stdout':'[{"ProcessName":"bimcraft","Id":42}]'})()
        with patch.object(qwen.os,'name','nt'), patch.object(qwen.subprocess,'run',return_value=result):
            with self.assertRaises(RuntimeError):
                qwen.ensure_cad_idle()

    def test_user_confirmed_leave_twenty_percent_allows_one_request(self):
        result=type('Result',(),{'returncode':0,'stdout':'[{"ProcessName":"bimcraft","Id":42}]'})()
        with patch.object(qwen.os,'name','nt'), patch.object(qwen.subprocess,'run',return_value=result):
            qwen.ensure_cad_idle(reserve_confirmed=True)
