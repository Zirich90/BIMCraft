import json
from pathlib import Path
import sys
import tempfile
import unittest
import zipfile

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import sync_memory


class MemoryExportTests(unittest.TestCase):
    def test_export_contains_wikilinks_and_preserves_user_state(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'graphify-out').mkdir()
            memory = root / 'vault'
            memory.mkdir()
            state = memory / '00_STATE.md'
            state.write_text('User-maintained state', encoding='utf-8')
            graph = {'nodes': [{'id': 'a', 'label': 'Alpha', 'source_file': 'src/a.rs'},
                               {'id': 'b', 'label': 'Beta'}],
                     'edges': [{'source': 'a', 'target': 'b', 'relation': 'uses', 'confidence': 'EXTRACTED'}]}
            (root / 'graphify-out/graph.json').write_text(json.dumps(graph), encoding='utf-8')
            self.assertEqual(sync_memory.export_graph(root, memory), (2, 1))
            note = memory / 'Graph/memory/memory__src_a__Alpha.md'
            self.assertIn('[[BIMCraft/Graph/memory/memory__unlocated__Beta|Beta]]', note.read_text(encoding='utf-8'))
            self.assertIn('NOT a fresh extraction', (root / 'graphify-out/GRAPH_REPORT.md').read_text())
            self.assertEqual(state.read_text(), 'User-maintained state')
            self.assertEqual(json.loads((memory / 'Graph/graph.json').read_text()), graph)

    def test_existing_regular_memory_directory_is_not_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / '.memory').mkdir()
            vault = root / 'vault'
            vault.mkdir()
            with self.assertRaises(ValueError):
                sync_memory.connect(root, vault)
            self.assertTrue((root / '.memory').is_dir())

    def test_rust_symbols_use_readable_crate_module_names(self):
        nodes = [
            {'id': 'carrier', 'label': 'CableCarrier', 'source_file': 'crates/core/src/model.rs'},
            {'id': 'ui', 'label': 'sync', 'source_file': 'crates/ui-egui/src/catalogue.rs'},
            {'id': 'cli', 'label': 'run', 'source_file': 'apps/bimcraft-cli/src/main.rs'},
            {'id': 'memory', 'label': 'execute', 'source_file': 'scripts/delegate_qwen.py'},
        ]
        paths = sync_memory.note_paths(nodes)
        self.assertEqual(paths['carrier'], 'core/core__model__CableCarrier.md')
        self.assertEqual(paths['ui'], 'ui/ui__catalogue__sync.md')
        self.assertEqual(paths['cli'], 'cli/cli__main__run.md')
        self.assertEqual(paths['memory'], 'memory/memory__scripts_delegate_qwen__execute.md')

    def test_collisions_are_case_insensitive_and_add_four_characters(self):
        nodes = [{'id': key, 'label': name, 'source_file': 'crates/core/src/model.rs'}
                 for key, name in [('a', 'check'), ('b', 'check'), ('c', 'CHECK')]]
        paths = sync_memory.note_paths(nodes)
        self.assertEqual(len({value.casefold() for value in paths.values()}), 3)
        self.assertEqual(paths['a'], 'core/core__model__check.md')
        self.assertRegex(paths['b'], r'^core/core__model__check__[0-9a-f]{4}\.md$')
        self.assertRegex(paths['c'], r'^core/core__model__CHECK__[0-9a-f]{4}\.md$')
        self.assertEqual(sync_memory.note_paths(list(reversed(nodes))), paths)

    def test_document_labels_do_not_look_like_wikilink_file_extensions(self):
        paths = sync_memory.note_paths([{'id': 'readme', 'label': 'README.md', 'source_file': 'README.md'}])
        self.assertEqual(paths['readme'], 'memory/memory__README__README_md.md')

    def test_legacy_hash_notes_are_backed_up_and_retired_without_touching_manual_notes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'graphify-out').mkdir()
            memory = root / 'vault'
            legacy = memory / 'Graph/Nodes'
            legacy.mkdir(parents=True)
            old = legacy / (sync_memory.legacy_node_name('a') + '.md')
            original = '---\ngraph_id: "a"\n---\nManual annotation preserved in backup.\n'
            old.write_text(original, encoding='utf-8')
            original_bytes = old.read_bytes()
            manual = legacy / 'my-note.md'
            manual.write_text('Do not touch', encoding='utf-8')
            graph = {'nodes': [{'id': 'a', 'label': 'CableCarrier', 'source_file': 'crates/core/src/model.rs'}], 'edges': []}
            (root / 'graphify-out/graph.json').write_text(json.dumps(graph), encoding='utf-8')
            sync_memory.export_graph(root, memory)
            self.assertFalse(old.exists())
            self.assertEqual(manual.read_text(), 'Do not touch')
            self.assertTrue((memory / 'Graph/core/core__model__CableCarrier.md').is_file())
            archives = list((root / 'target/memory-graph-backups').glob('*.zip'))
            self.assertEqual(len(archives), 1)
            with zipfile.ZipFile(archives[0]) as archive:
                self.assertEqual(archive.read('Nodes/' + old.name), original_bytes)

    def test_renamed_nodes_retire_old_paths_and_leave_manual_collisions_intact(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'graphify-out').mkdir()
            memory = root / 'vault'
            folder = memory / 'Graph/core'
            folder.mkdir(parents=True)
            manual = folder / 'core__model__Cable.md'
            manual.write_bytes(b'User note, not generated')
            source = root / 'graphify-out/graph.json'
            graph = {'nodes': [{'id': 'a', 'label': 'Cable', 'source_file': 'crates/core/src/model.rs'}], 'edges': []}
            source.write_text(json.dumps(graph), encoding='utf-8')
            sync_memory.export_graph(root, memory)
            manifest = memory / 'Graph/.generated-files.json'
            old = memory / 'Graph' / json.loads(manifest.read_text())['files']['a']
            self.assertNotEqual(old, manual)
            self.assertTrue(old.is_file())
            # An unchanged refresh creates neither renamed notes nor backups.
            sync_memory.export_graph(root, memory)
            self.assertFalse((root / 'target/memory-graph-backups').exists())
            graph['nodes'][0]['label'] = 'Carrier'
            source.write_text(json.dumps(graph), encoding='utf-8')
            sync_memory.export_graph(root, memory)
            self.assertFalse(old.exists())
            self.assertTrue((folder / 'core__model__Carrier.md').is_file())
            self.assertEqual(manual.read_bytes(), b'User note, not generated')
            archives = list((root / 'target/memory-graph-backups').glob('*.zip'))
            with zipfile.ZipFile(archives[0]) as archive:
                self.assertIn(old.relative_to(memory / 'Graph').as_posix(), archive.namelist())


if __name__ == '__main__':
    unittest.main()
