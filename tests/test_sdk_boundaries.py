"""Regression tests for SDK dependency and source architecture gates."""

import contextlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from unittest import mock

SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "check-sdk-boundaries.py"
SPEC = importlib.util.spec_from_file_location("sdk_boundaries", SCRIPT)
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


def metadata(edges):
    names = set(edges)
    names.update(target for dependencies in edges.values() for target, _ in dependencies)
    return {
        "packages": [{"id": name, "name": name} for name in names],
        "resolve": {"nodes": [
            {"id": name, "deps": [
                {"name": "renamed_dependency", "pkg": target, "dep_kinds": [{"kind": kind}]}
                for target, kind in edges.get(name, [])
            ]} for name in names
        ]},
    }


class MetadataTests(unittest.TestCase):
    def test_cargo_utf8_metadata_is_independent_of_windows_code_page(self):
        payload = json.dumps({**metadata({}), "description": "Unicode: é" + chr(0x81)}, ensure_ascii=False).encode("utf-8")
        run = CHECK.subprocess.run

        def cargo_output(_command, **kwargs):
            return run([sys.executable, "-c", f"import sys; sys.stdout.buffer.write({payload!r})"], **kwargs)

        with (mock.patch.object(CHECK.subprocess, "run", side_effect=cargo_output),
              mock.patch.object(CHECK.subprocess, "_text_encoding", return_value="cp1252"),
              mock.patch.object(CHECK.sys, "argv", ["check-sdk-boundaries.py"]),
              contextlib.redirect_stdout(io.StringIO()), contextlib.redirect_stderr(io.StringIO())):
            self.assertEqual(CHECK.main(), 0)


class DependencyTests(unittest.TestCase):
    def test_downward_dependencies_pass(self):
        graph = metadata({"papo-ffi": [("papo-sdk-ui", None)],
                          "papo-sdk-ui": [("papo-sdk", None)],
                          "papo-sdk": [("papo-api", None)], "papo-api": []})
        self.assertEqual(CHECK.dependency_violations(graph), [])

    def test_renamed_upward_dependency_fails(self):
        graph = metadata({"papo-api": [("papo-sdk-ui", None)]})
        self.assertTrue(CHECK.dependency_violations(graph))

    def test_ui_cannot_bypass_core_public_api(self):
        graph = metadata({"papo-sdk-ui": [("papo-api", None)]})
        self.assertTrue(CHECK.dependency_violations(graph))

    def test_transitive_ui_dependency_fails(self):
        graph = metadata({"papo-sdk": [("third-party", None)],
                          "third-party": [("gpui", None)]})
        self.assertTrue(CHECK.dependency_violations(graph))

    def test_legacy_dependency_fails(self):
        graph = metadata({"papo-sdk": [("papo-core", None)]})
        self.assertTrue(CHECK.dependency_violations(graph))

    def test_test_server_is_not_a_production_dependency(self):
        graph = metadata({"papo-sdk": [("papo-test-server", None)]})
        self.assertTrue(CHECK.dependency_violations(graph))

    def test_uniffi_is_allowed_only_at_binding_boundary(self):
        self.assertEqual(CHECK.dependency_violations(metadata({"papo-ffi": [("uniffi", None)]})), [])
        self.assertTrue(CHECK.dependency_violations(metadata({"papo-sdk": [("uniffi", None)]})))

    def test_renamed_direct_and_transitive_sdk_test_dependencies_are_checked(self):
        for target in ["gpui", "papo-core", "uniffi"]:
            for edges in [
                {"papo-sdk": [(target, "dev")]},
                {"papo-sdk": [("helper", "dev")], "helper": [(target, None)]},
            ]:
                with self.subTest(edges=edges):
                    self.assertTrue(CHECK.dependency_violations(metadata(edges)))

    def test_third_party_dev_dependencies_are_not_built(self):
        graph = metadata({"papo-sdk": [("helper", None)], "helper": [("gpui", "dev")]})
        self.assertEqual(CHECK.dependency_violations(graph), [])

    def test_test_only_loopback_backend_is_allowed(self):
        graph = metadata({"papo-sdk": [("papo-test-server", "dev")]})
        self.assertEqual(CHECK.dependency_violations(graph), [])


class SourceTests(unittest.TestCase):
    def test_platform_types_and_macros_fail(self):
        for source in ["use gpui::Entity;", "use uniffi::Object;", "tr!(\"Offline\")"]:
            with self.subTest(source=source):
                self.assertTrue(CHECK.source_violations(source, "papo-sdk"))

    def test_string_shaped_errors_fail(self):
        for source in ['pub error: Option<String>', 'fn f() -> Result<(), String> {}',
                       'Err("Offline".into())', "pub status: &'static str"]:
            with self.subTest(source=source):
                self.assertTrue(CHECK.source_violations(source, "papo-sdk"))

    def test_diagnostics_protocol_literals_and_comments_pass(self):
        source = '''//! gpui::Entity belongs to the client.
        /* outer /* nested */ uniffi::Object */
        #[error("diagnostic description")]
        enum Failure { Invalid }
        const WIRE: &str = r#"https://example.invalid/gpui::Entity"#;
        Err(Failure::Invalid)
        '''
        self.assertEqual(CHECK.source_violations(source, "papo-sdk"), [])

    def test_quotes_in_block_comments_cannot_hide_live_code(self):
        for comment in ['/* " */', '/* outer /* " */ nested */', '/* r#" */']:
            source = comment + ' pub fn f() -> Result<(), String> { Err("Offline".into()) }'
            with self.subTest(comment=comment):
                self.assertTrue(CHECK.source_violations(source, "papo-sdk"))

    def test_comment_markers_in_char_and_raw_literals_do_not_hide_code(self):
        for literal in ["'\\\"'", "'/'", 'r##"/* // \" "##', 'br#"/*"#']:
            source = f'let text = {literal}; use gpui::Entity;'
            with self.subTest(literal=literal):
                self.assertTrue(CHECK.source_violations(source, "papo-sdk"))

    def test_uniffi_is_allowed_but_widgets_are_not_in_bindings(self):
        self.assertEqual(CHECK.source_violations("use uniffi::Object;", "papo-ffi"), [])
        self.assertTrue(CHECK.source_violations("use gpui::Entity;", "papo-ffi"))

    def test_wire_error_payload_strings_are_not_ui_copy(self):
        self.assertEqual(CHECK.source_violations("pub error: String", "papo-api"), [])


if __name__ == "__main__":
    unittest.main()
