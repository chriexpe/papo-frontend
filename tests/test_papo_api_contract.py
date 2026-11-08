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
            original = yaml.safe_load((original_api / 'protocol/openapi.yml').read_text())
            (api / 'protocol/openapi.yml').write_text(yaml.safe_dump(original))
            contract.API = api
            try:
                before = contract.generate()
                mutate(original)
                (api / 'protocol/openapi.yml').write_text(yaml.safe_dump(original))
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

    def test_removed_implemented_route_fails(self):
        with self.assertRaises(KeyError):
            self.changed(lambda doc: doc['paths'].pop('/auth/login'))


if __name__ == '__main__':
    unittest.main()
