#!/usr/bin/env python3
"""Generate protocol records from the pinned contract; check drift without network."""
import argparse
import json
import pathlib
import re
import subprocess
import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
API = ROOT / 'crates/sdk/api'
ROUTES = {
    '/auth/register': ['post'], '/auth/login': ['post'],
    '/auth/login_server': ['post'], '/auth/whoami': ['get'],
    '/auth/logout': ['post'], '/auth/refresh': ['post'],
    '/auth/connected_devices': ['get'], '/auth/drop_connection': ['post'],
    '/auth/password_reset': ['post'], '/server': ['get', 'post'],
    '/channels': ['get', 'post'], '/channels/{channel_id}': ['put', 'delete'],
    '/channels/{channel_id}/messages': ['get'], '/messages': ['post'],
    '/messages/{message_id}': ['put', 'delete'],
}


def snake(name):
    return re.sub(r'(?<!^)(?=[A-Z])', '_', name).lower()


def generate():
    spec = yaml.safe_load((API / 'protocol/openapi.yml').read_text())
    schemas = spec['components']['schemas']
    pending = {'DirectConversation', 'Embed', 'Error'}
    referenced = {}
    contract = {}
    def refs(node):
        if isinstance(node, dict):
            if '$ref' in node and node['$ref'].startswith('#/components/schemas/'):
                pending.add(node['$ref'].split('/')[-1])
            elif '$ref' in node and node['$ref'].startswith('#/') and node['$ref'] not in referenced:
                target = spec
                for part in node['$ref'][2:].split('/'):
                    target = target[part]
                referenced[node['$ref']] = target
                refs(target)
            for value in node.values():
                refs(value)
        elif isinstance(node, list):
            for value in node:
                refs(value)
    for path, methods in ROUTES.items():
        contract[path] = {method: spec['paths'][path][method] for method in methods}
        refs(contract[path])
    output = {}
    generated = {}
    def rust_type(node, name):
        if '$ref' in node:
            return 'super::' + node['$ref'].split('/')[-1].replace('Error', 'Problem')
        if 'allOf' in node:
            assert len(node['allOf']) == 1
            return rust_type(node['allOf'][0], name)
        if node.get('format') == 'binary':
            return 'Vec<u8>'
        kind = node.get('type', 'object')
        if kind == 'array':
            return 'Vec<' + rust_type(node['items'], name + 'Item') + '>'
        if kind == 'object':
            if 'properties' in node:
                record(name, node)
                return 'super::' + name
            return 'std::collections::BTreeMap<String, serde_json::Value>'
        return {'string': 'String', 'integer': 'i64', 'number': 'f64', 'boolean': 'bool'}[kind]
    def record(name, node):
        if node.get('type', 'object') != 'object':
            generated[name] = f'/// Scalar wire value for `{name}`.\npub type {name} = {rust_type(node, name)};\n'
            return
        lines = [f'/// Wire record for `{name}`. Optional fields follow the pinned `OpenAPI` contract.',
                 '#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]',
                 f'pub struct {name} {{']
        for field, schema in node.get('properties', {}).items():
            typ = rust_type(schema, name + ''.join(x.title() for x in field.split('_')))
            optional = field not in node.get('required', []) or schema.get('nullable', False)
            lines += [f'    /// Backend `{field}` field.' + (' Timestamp strings are preserved verbatim.' if schema.get('format') == 'date-time' else '')]
            if optional:
                typ = f'Option<{typ}>'
                lines += ['    #[serde(default, skip_serializing_if = "Option::is_none")]']
            if snake(field) != field:
                lines += [f'    #[serde(rename = "{field}")]']
            lines += [f'    pub r#{snake(field)}: {typ},']
        lines += ['}']
        generated[name] = '\n'.join(lines) + '\n'
    record('ChannelList', spec['paths']['/channels']['get']['responses']['200']['content']['application/json']['schema'])
    seen = set()
    while pending - seen:
        name = sorted(pending - seen)[0]
        seen.add(name)
        refs(schemas[name])
        record(name.replace('Error', 'Problem'), schemas[name])
    for name, body in generated.items():
        output[API / 'src/wire' / (snake(name) + '.rs')] = (
            '//! Generated wire record; transport and policy belong to other modules.\n\n' + body)
    modules = '\n'.join(f'mod {snake(name)};\npub use {snake(name)}::*;' for name in sorted(generated))
    output[API / 'src/wire/mod.rs'] = '//! Generated protocol records; owns serialization, never client state.\n\n' + modules + '\n'
    # Include constraints, parameters, error responses and security, not only field names.
    contract['references'] = referenced
    contract['security'] = spec.get('security', [])
    contract['securitySchemes'] = spec['components'].get('securitySchemes', {})
    contract['schemas'] = {name: schemas[name] for name in sorted(seen)}
    output[API / 'protocol/implemented.json'] = json.dumps(contract, indent=2, ensure_ascii=False) + '\n'
    return output


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    failed = False
    for path, content in generate().items():
        if path.suffix == '.rs':
            content = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'], input=content,
                                     text=True, capture_output=True, check=True).stdout
        if args.write:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
        elif not path.exists() or path.read_text() != content:
            print(f'Protocol drift: {path.relative_to(ROOT)}')
            failed = True
    if failed:
        raise SystemExit('Review wire changes and regenerate deliberately with --write.')


if __name__ == '__main__':
    main()
