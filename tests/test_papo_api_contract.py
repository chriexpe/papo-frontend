"""Regression checks prove that route/schema changes alter the checked wire contract."""
import importlib.util
import pathlib
import tempfile
import unittest
import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('papo_api_contract', ROOT / 'scripts/papo-api-contract.py')
contract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)


class ProtocolDrift(unittest.TestCase):
    def changed(self, mutate):
        original_api = contract.API
        with tempfile.TemporaryDirectory() as directory:
            api = pathlib.Path(directory)
            (api / 'protocol').mkdir()
            original = yaml.safe_load((original_api / 'protocol/openapi.yml').read_text(encoding='utf-8'))
            (api / 'protocol/openapi.yml').write_text(yaml.safe_dump(original), encoding='utf-8')
            contract.API = api
            try:
                before = contract.generate()
                mutate(original)
                (api / 'protocol/openapi.yml').write_text(yaml.safe_dump(original), encoding='utf-8')
                after = contract.generate()
                return before, after
            finally:
                contract.API = original_api

    def test_response_field_type_changes_wire_record(self):
        before, after = self.changed(lambda doc: doc['components']['schemas']['Message']['properties']['content'].update(type='integer'))
        key = next(path for path in before if path.name == 'message.rs')
        self.assertNotEqual(before[key], after[key])
        self.assertIn('Option<i64>', after[key])

    def test_required_request_field_changes_wire_record(self):
        before, after = self.changed(lambda doc: doc['components']['schemas']['LoginRequest']['required'].remove('password'))
        key = next(path for path in before if path.name == 'login_request.rs')
        self.assertNotEqual(before[key], after[key])

    def test_history_query_constraint_changes_contract(self):
        def mutate(doc):
            parameters = doc['paths']['/channels/{channel_id}/messages']['get']['parameters']
            next(p for p in parameters if p.get('name') == 'limit')['schema']['maximum'] = 50
        before, after = self.changed(mutate)
        key = next(path for path in before if path.name == 'implemented.json')
        self.assertNotEqual(before[key], after[key])

    def test_inline_read_body_changes_wire_record(self):
        def mutate(doc):
            schema = doc['paths']['/channels/{channel_id}/read']['put']['requestBody']['content']['application/json']['schema']
            schema['properties']['message_id']['type'] = 'integer'
        before, after = self.changed(mutate)
        key = next(path for path in before if path.name == 'read_channel_request.rs')
        self.assertNotEqual(before[key], after[key])
        self.assertIn('i64', after[key])

    def test_composite_embed_fields_change_wire_record(self):
        def mutate(doc):
            properties = doc['components']['schemas']['EmbedWithImage']['allOf'][1]['properties']
            properties['image_data']['type'] = 'integer'
        before, after = self.changed(mutate)
        key = next(path for path in before if path.name == 'embed_with_image.rs')
        self.assertIn('r#image_data: Option<String>', before[key])
        self.assertIn('r#image_data: Option<i64>', after[key])
        self.assertNotEqual(before[key], after[key])

    def test_route_inventory_covers_pinned_spec(self):
        document = yaml.safe_load((contract.API / 'protocol/openapi.yml').read_text(encoding='utf-8'))
        methods = {'get', 'post', 'put', 'patch', 'delete'}
        expected = {(path, method) for path, operations in document['paths'].items()
                    for method in operations if method in methods}
        registered = {(path, method) for path, operations in contract.ROUTES.items()
                      for method in operations}
        self.assertEqual(expected, registered)

    def test_removed_implemented_route_fails(self):
        with self.assertRaises(KeyError):
            self.changed(lambda doc: doc['paths'].pop('/auth/login'))


if __name__ == '__main__':
    unittest.main()
