"""Offline regression tests for the pinned backend wire contract checker."""
import contextlib
import copy
import importlib.util
import io
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('papo_api_contract', ROOT / 'scripts/papo-api-contract.py')
contract = importlib.util.module_from_spec(spec)
spec.loader.exec_module(contract)


def document():
    paths = {path: {method: {} for method in methods} for path, methods in contract.ROUTES.items()}
    inline = [
        ('/users/profile_batch', 'post', 'responses'), ('/users', 'get', 'responses'),
        ('/users/blocks', 'get', 'responses'), ('/users/settings', 'put', 'requestBody'),
        ('/roles', 'get', 'responses'), ('/channels', 'get', 'responses'),
        ('/channels/{channel_id}/permissions/{role_id}', 'put', 'responses'),
        ('/channels/{channel_id}/read', 'put', 'requestBody'),
        ('/emojis', 'get', 'responses'), ('/webhooks', 'get', 'responses'),
    ]
    for path, method, section in inline:
        schema = {'type': 'object', 'properties': {'value': {'type': 'string'}}}
        body = {'content': {'application/json': {'schema': schema}}}
        paths[path][method][section] = {'200': body} if section == 'responses' else body
    paths['/auth/login']['post']['requestBody'] = {
        'content': {'application/json': {'schema': {'$ref': '#/components/schemas/LoginRequest'}}}}
    paths['/messages']['post']['responses'] = {
        '200': {'content': {'application/json': {'schema': {'$ref': '#/components/schemas/Composite'}}}}}
    return {'paths': paths, 'components': {'schemas': {
        'DirectConversation': {'type': 'object'}, 'Embed': {'type': 'object'},
        'Error': {'type': 'object'},
        'LoginRequest': {'type': 'object', 'required': ['password'],
                         'properties': {'password': {'type': 'string'}}},
        'Composite': {'allOf': [{'$ref': '#/components/schemas/Base'},
                                {'properties': {'image_data': {'type': 'string'}}}]},
        'Base': {'type': 'object', 'properties': {'id': {'type': 'integer'}}},
    }}}


class ProtocolDrift(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = pathlib.Path(self.temp.name)
        self.api = root / 'api'
        (self.api / 'protocol').mkdir(parents=True)
        self.backend = root / 'backend'
        self.backend.mkdir()
        (self.backend / 'openapi.yml').write_text(yaml.safe_dump(document()), encoding='utf-8')
        env = dict(os.environ, GIT_CONFIG_NOSYSTEM='1', GIT_CONFIG_GLOBAL=os.devnull,
                   GIT_AUTHOR_NAME='Fixture', GIT_AUTHOR_EMAIL='fixture@example.invalid',
                   GIT_COMMITTER_NAME='Fixture', GIT_COMMITTER_EMAIL='fixture@example.invalid',
                   GIT_AUTHOR_DATE='2000-01-01T00:00:00+0000', GIT_COMMITTER_DATE='2000-01-01T00:00:00+0000')
        for command in (['init', '-q'], ['add', 'openapi.yml'], ['commit', '-qm', 'fixture']):
            subprocess.run(['git', '-C', str(self.backend), *command], env=env, check=True)
        head = subprocess.run(['git', '-C', str(self.backend), 'rev-parse', 'HEAD'],
                              env=env, text=True, capture_output=True, check=True).stdout.strip()
        (self.api / 'protocol/backend-commit').write_text(head + '\n', encoding='utf-8')
        self.patch = mock.patch.object(contract, 'API', self.api)
        self.patch.start()
        self.addCleanup(self.patch.stop)

    def run_checker(self, *args):
        with mock.patch.object(sys, 'argv', ['checker', *args]), contextlib.redirect_stdout(io.StringIO()):
            contract.main()

    def test_required_backend_and_matching_checkout(self):
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            self.run_checker()
        self.run_checker('--backend', str(self.backend), '--write')
        self.run_checker('--backend', str(self.backend))
        self.assertFalse((self.api / 'protocol/implemented.json').exists())

    def test_wrong_head_and_non_checkout_rejected(self):
        (self.api / 'protocol/backend-commit').write_text('0' * 40, encoding='utf-8')
        with self.assertRaisesRegex(SystemExit, 'does not match pinned commit'):
            self.run_checker('--backend', str(self.backend))
        with self.assertRaisesRegex(SystemExit, 'checkout'):
            self.run_checker('--backend', str(self.api))

    def test_extra_and_missing_wire_files_detected(self):
        with self.assertRaises(SystemExit):
            self.run_checker('--backend', str(self.backend))
        self.run_checker('--backend', str(self.backend), '--write')
        (self.api / 'src/wire/login_request.rs').write_text('drift', encoding='utf-8')
        with self.assertRaises(SystemExit):
            self.run_checker('--backend', str(self.backend))
        self.run_checker('--backend', str(self.backend), '--write')
        stale = self.api / 'src/wire/obsolete.rs'
        stale.write_text('// stale\n', encoding='utf-8')
        with self.assertRaises(SystemExit):
            self.run_checker('--backend', str(self.backend))
        with self.assertRaises(SystemExit):
            self.run_checker('--backend', str(self.backend), '--write')
        self.assertTrue(stale.exists())

    def test_missing_and_unregistered_routes_rejected(self):
        for mutate in (lambda doc: doc['paths'].pop('/auth/login'),
                       lambda doc: doc['paths']['/health'].update(options={}),
                       lambda doc: doc['paths'].update({'/new': {'get': {}}})):
            changed = document()
            mutate(changed)
            with self.assertRaisesRegex(ValueError, 'Route coverage mismatch'):
                contract.generate(changed)

    def test_required_inline_and_composite_fields_change_output(self):
        original = document()
        before = contract.generate(original)
        changed = copy.deepcopy(original)
        changed['components']['schemas']['LoginRequest']['required'].remove('password')
        changed['paths']['/channels/{channel_id}/read']['put']['requestBody']['content']['application/json']['schema']['properties']['value']['type'] = 'integer'
        changed['components']['schemas']['Composite']['allOf'][1]['properties']['image_data']['type'] = 'integer'
        after = contract.generate(changed)
        for name in ('login_request.rs', 'read_channel_request.rs', 'composite.rs'):
            path = self.api / 'src/wire' / name
            self.assertNotEqual(before[path], after[path])
        self.assertIn('Option<String>', after[self.api / 'src/wire/login_request.rs'])
        self.assertIn('Option<i64>', after[self.api / 'src/wire/composite.rs'])
        self.assertIn('i64', after[self.api / 'src/wire/read_channel_request.rs'])

    def test_backend_openapi_is_read_directly(self):
        self.run_checker('--backend', str(self.backend), '--write')
        changed = document()
        changed['components']['schemas']['LoginRequest']['properties']['password']['type'] = 'integer'
        (self.backend / 'openapi.yml').write_text(yaml.safe_dump(changed), encoding='utf-8')
        with self.assertRaises(SystemExit):
            self.run_checker('--backend', str(self.backend))


if __name__ == '__main__':
    unittest.main()
