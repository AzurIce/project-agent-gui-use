import unittest

from notist_protocol import respond


class ProtocolExtensionTests(unittest.TestCase):
    def setUp(self):
        self.registry = {
            "methods": [{
                "name": "app.list", "docs": "List **apps**.\n\n```json\n{}\n```",
            }],
            "events": [{
                "name": "notification.posted", "docs": "A *notification*.", "merge": "keep",
            }],
        }

    def test_compile_preserves_markdown_and_leaves_tables_to_notc(self):
        documents = {
            "protocol/methods.md": "# Methods\n\n<!-- protocol:methods -->\n",
            "protocol/events.md": "# Events\n\n<!-- protocol:events -->\n",
            "other.md": "untouched",
        }
        result = respond({"version": 2, "event": "before_compile", "documents": documents}, self.registry)
        source = result["documents"]["protocol/methods.md"]
        self.assertIn("List **apps**.\n\n```json\n{}\n```", source)
        self.assertIn('#protocol.method_fields(0, "params")', source)
        self.assertNotIn("<!-- protocol:methods -->", source)
        self.assertEqual(result["documents"]["other.md"], "untouched")
        self.assertIn("<!-- protocol:methods -->", documents["protocol/methods.md"])

    def test_template_data_preserves_other_extensions_data(self):
        result = respond({"version": 2, "event": "before_template", "data": {"book": {"title": "Book"}}}, self.registry)
        self.assertEqual(result["data"]["protocol"], self.registry)
        self.assertEqual(result["data"]["book"], {"title": "Book"})

    def test_missing_markers_and_unknown_versions_fail(self):
        with self.assertRaises(ValueError):
            respond({"version": 2, "event": "before_compile", "documents": {
                "protocol/methods.md": "# Missing marker", "protocol/events.md": "<!-- protocol:events -->",
            }}, self.registry)
        with self.assertRaises(ValueError):
            respond({"version": 0, "event": "before_compile"}, self.registry)


if __name__ == "__main__":
    unittest.main()
