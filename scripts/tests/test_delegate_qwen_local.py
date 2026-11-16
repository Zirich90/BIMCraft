import unittest
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import delegate_qwen


class DelegateQwenModelResultTests(unittest.TestCase):
    def test_plain_json_object_is_returned(self):
        response = {
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {"content": '{"a": 1}'},
                }
            ]
        }
        self.assertEqual(delegate_qwen.model_result(response), {"a": 1})

    def test_fenced_json_object_is_returned(self):
        response = {
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {"content": '```json\n{"a": 1}\n```'},
                }
            ]
        }
        self.assertEqual(delegate_qwen.model_result(response), {"a": 1})

    def test_json_array_raises_value_error(self):
        response = {
            "choices": [
                {
                    "finish_reason": "stop",
                    "message": {"content": '[1, 2]'},
                }
            ]
        }
        with self.assertRaises(ValueError):
            delegate_qwen.model_result(response)


if __name__ == "__main__":
    unittest.main()
