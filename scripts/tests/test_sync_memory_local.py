import unittest
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import sync_memory


class TestNotePaths(unittest.TestCase):
    def test_gpu_mesh_build(self):
        nodes = [{"id": "gpu", "label": "Mesh::build", "source_file": "crates/core/src/view3d/gpu.rs"}]
        result = sync_memory.note_paths(nodes, reserved=())
        self.assertEqual(result["gpu"], "core/core__view3d_gpu__Mesh_build.md")

    def test_cli_main_write(self):
        nodes = [{"id": "apps_bimcraft_cli_src_main_rs_write", "label": "Write", "source_file": ""}]
        result = sync_memory.note_paths(nodes, reserved=())
        self.assertEqual(result["apps_bimcraft_cli_src_main_rs_write"], "cli/cli__main__Write.md")

    def test_reserved_collision(self):
        nodes = [{"id": "a", "label": "check", "source_file": "crates/core/src/model.rs"}]
        reserved = ["core/core__model__check.md"]
        result = sync_memory.note_paths(nodes, reserved=reserved)
        path = result["a"]
        prefix = "core/core__model__check__"
        self.assertTrue(path.startswith(prefix))
        self.assertTrue(path.endswith(".md"))
        suffix = path[len(prefix):-3]
        self.assertEqual(len(suffix), 4)
        self.assertTrue(all(c in "0123456789abcdef" for c in suffix))


if __name__ == "__main__":
    unittest.main()
