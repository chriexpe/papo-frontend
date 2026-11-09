#!/usr/bin/env python3
"""Generate protocol records from the pinned contract; check drift without network."""
import argparse
import pathlib
import re
import subprocess
import yaml

ROOT = pathlib.Path(__file__).resolve().parents[1]
API = ROOT / 'crates/sdk/api'
ROUTES = {
    '/health': ['get'],
    '/auth/register': ['post'], '/auth/login': ['post'],
    '/auth/login_server': ['post'], '/auth/whoami': ['get'],
    '/auth/logout': ['post'], '/auth/refresh': ['post'],
    '/auth/connected_devices': ['get'], '/auth/drop_connection': ['post'],
    '/auth/password_reset': ['post'],
    '/users': ['get'], '/users/{user_id}/profile': ['get'],
    '/users/profile_batch': ['post'], '/users/user_summary_batch': ['post'],
    '/users/blocks': ['get'], '/users/{user_id}/block': ['post', 'delete'],
    '/users/settings': ['put'], '/users/{user_id}': ['put'],
    '/users/{user_id}/avatar': ['get', 'put'], '/users/{user_id}/banner': ['put'],
    '/users/{user_id}/status': ['put'], '/users/{user_id}/password': ['put'],
    '/users/{user_id}/ban': ['put'], '/users/{user_id}/reset': ['post'],
    '/users/me/push-device': ['put', 'delete'],
    '/users/{user_id}/roles': ['post'], '/users/{user_id}/roles/{role_id}': ['delete'],
    '/users/{user_id}/notifications': ['get'], '/users/{user_id}/read_notification': ['put'],
    '/dms': ['get', 'post'], '/dms/{dm_id}': ['get', 'delete'],
    '/server': ['get', 'post', 'put', 'patch'],
    '/roles': ['get', 'post'], '/roles/{role_id}': ['put', 'delete'],
    '/channels': ['get', 'post'], '/channels/{channel_id}': ['put', 'delete'],
    '/channels/{channel_id}/change_position': ['put'],
    '/channels/{channel_id}/permissions': ['get'],
    '/channels/{channel_id}/permissions/{role_id}': ['put'],
    '/channels/{channel_id}/role/{role_id}': ['delete'],
    '/channels/{channel_id}/user/{user_id}/settings': ['post'],
    '/channels/{channel_id}/read': ['put'], '/voice/ice-servers': ['get'],
    '/channels/{channel_id}/messages': ['get'], '/messages': ['post'],
    '/messages/{message_id}': ['put', 'delete'],
    '/channels/{channel_id}/messages/{message_id}/pin': ['post', 'delete'],
    '/channels/{channel_id}/pinned': ['get'],
    '/channels/{channel_id}/messages/{message_id}/reactions': ['get', 'post', 'delete'],
    '/emojis': ['get', 'post'], '/emojis/{emoji_id}': ['delete'],
    '/attachments/{file_id}': ['get'], '/attachments/{file_id}/thumbnail': ['get'],
    '/media/{sha_hash}': ['get'], '/embeds/{embed_id}': ['get'],
    '/embeds/{embed_id}/video': ['get'], '/search': ['post'],
    '/admin/audit-logs': ['get'], '/webhooks': ['get', 'post'],
    '/webhooks/{webhook_id}': ['patch', 'delete'],
    '/webhooks/{webhook_id}/avatar': ['get'],
    '/webhooks/{webhook_id}/messages': ['post'],
    '/webhooks/{webhook_id}/messages/{message_id}': ['put', 'delete'],
    '/ws': ['get'],
}


def snake(name):
    return re.sub(r'(?<!^)(?=[A-Z])', '_', name).lower()


def generate(spec):
    schemas = spec['components']['schemas']
    methods = {'get', 'put', 'post', 'delete', 'options', 'head', 'patch', 'trace'}
    actual = {(path, method) for path, operations in spec['paths'].items()
              for method in operations if method in methods}
    registered = {(path, method) for path, operations in ROUTES.items() for method in operations}
    if actual != registered:
        missing = sorted(registered - actual)
        extra = sorted(actual - registered)
        raise ValueError(f'Route coverage mismatch: missing={missing}, unregistered={extra}')
    pending = {'DirectConversation', 'Embed', 'Error'}
    referenced = set()
    def refs(node):
        if isinstance(node, dict):
            if '$ref' in node and node['$ref'].startswith('#/components/schemas/'):
                pending.add(node['$ref'].split('/')[-1])
            elif '$ref' in node and node['$ref'].startswith('#/') and node['$ref'] not in referenced:
                target = spec
                for part in node['$ref'][2:].split('/'):
                    target = target[part]
                referenced.add(node['$ref'])
                refs(target)
            for value in node.values():
                refs(value)
        elif isinstance(node, list):
            for value in node:
                refs(value)
    for path, route_methods in ROUTES.items():
        for method in route_methods:
            refs(spec['paths'][path][method])
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
        if 'allOf' in node:
            combined = {'type': 'object', 'properties': {}, 'required': []}
            for part in node['allOf']:
                if '$ref' in part:
                    part = schemas[part['$ref'].split('/')[-1]]
                combined['properties'].update(part.get('properties', {}))
                combined['required'].extend(part.get('required', []))
            node = combined
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
    inline = {
        'ProfileBatchResponse': ('/users/profile_batch', 'post', 'responses', '200'),
        'MemberListResponse': ('/users', 'get', 'responses', '200'),
        'BlockListResponse': ('/users/blocks', 'get', 'responses', '200'),
        'UpdateUserSettingsRequest': ('/users/settings', 'put', 'requestBody', None),
        'RoleListResponse': ('/roles', 'get', 'responses', '200'),
        'ChannelList': ('/channels', 'get', 'responses', '200'),
        'UpdateChannelPermissionsResponse': ('/channels/{channel_id}/permissions/{role_id}', 'put', 'responses', '200'),
        'ReadChannelRequest': ('/channels/{channel_id}/read', 'put', 'requestBody', None),
        'EmojiListResponse': ('/emojis', 'get', 'responses', '200'),
        'WebhookListResponse': ('/webhooks', 'get', 'responses', '200'),
    }
    for name, (path, method, section, status) in inline.items():
        node = spec['paths'][path][method][section]
        if status is not None:
            node = node[status]
        record(name, node['content']['application/json']['schema'])
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
    output[API / 'src/wire/mod.rs'] = ('//! Generated protocol records; owns serialization, never client state.\n\n'
                                      'pub(crate) mod routes;\n' + modules + '\n')
    output[API / 'src/wire/routes.rs'] = route_table(spec)
    return output


def route_parameters(spec, operation):
    names = []
    for parameter in operation.get('parameters', []):
        if '$ref' in parameter:
            parameter = spec['components']['parameters'][parameter['$ref'].split('/')[-1]]
        if parameter.get('in') == 'query':
            names.append(parameter['name'])
    return names


def route_table(spec):
    query_names = sorted({name for path in ROUTES for method in ROUTES[path]
                          for name in route_parameters(spec, spec['paths'][path][method])})
    lines = ['//! Generated REST operation table; transport and policy belong to other modules.',
             '',
             '#![allow(dead_code, reason = "generated operation metadata is checked by drift tests")]',
             '']
    for name in query_names:
        lines += [f'/// Backend query parameter `{name}`.', f'pub(crate) const {name.upper()}: &str = "{name}";', '']
    lines += ['/// One pinned backend operation and its transport shape.',
              'pub(crate) struct Route {',
              '    /// HTTP method.',
              '    pub(crate) method: reqwest::Method,',
              '    /// Path template; `{name}` placeholders are filled by [`Route::url`].',
              '    pub(crate) path: &\'static str,',
              '    /// Accepted query parameter names.',
              '    pub(crate) query: &\'static [&\'static str],',
              '    /// Request body content type, when the operation accepts a body.',
              '    pub(crate) request_body: Option<&\'static str>,',
              '    /// Successful HTTP statuses.',
              '    pub(crate) success: &\'static [u16],',
              '}',
              '',
              'impl Route {',
              '    /// Expands path placeholders in their template order. Missing values are left empty.',
              '    pub(crate) fn url(&self, values: &[String]) -> String {',
              '        let mut url = String::new();',
              '        let mut rest = self.path;',
              '        let mut values = values.iter();',
              '        while let Some(start) = rest.find(\'{\') {',
              '            url.push_str(&rest[..start]);',
              '            let Some(end) = rest[start..].find(\'}\') else { break };',
              '            let end = start + end;',
              '            if let Some(value) = values.next() { url.push_str(value); }',
              '            rest = &rest[end + 1..];',
              '        }',
              '        url.push_str(rest);',
              '        url',
              '    }',
              '}',
              '']
    for path, methods in ROUTES.items():
        for method in methods:
            operation = spec['paths'][path][method]
            name = method.upper() + '_' + '_'.join(re.findall(r'[A-Za-z0-9]+', path)).upper()
            query = ', '.join(name.upper() for name in route_parameters(spec, operation))
            body = operation.get('requestBody', {}).get('content', {})
            if len(body) > 1:
                raise ValueError(f'Multiple request content types for {method.upper()} {path}')
            request_body = next(iter(body), None)
            success = ', '.join(str(status) for status in sorted(
                int(code) for code in operation.get('responses', {})
                if code.isdigit() and 200 <= int(code) < 300))
            body_field = f'    request_body: Some("{request_body}"),' if request_body else '    request_body: None,'
            lines += [f'/// `{method.upper()} {path}`.',
                      f'pub(crate) const {name}: Route = Route {{',
                      f'    method: reqwest::Method::{method.upper()},',
                      f'    path: "{path}",',
                      f'    query: &[{query}],',
                      body_field,
                      f'    success: &[{success}],',
                      '};',
                      '']
    lines += ['#[cfg(test)]',
              'mod tests {',
              '    //! Verifies path-template expansion without network access.',
              '',
              '    use super::*;',
              '',
              '    #[test]',
              '    fn path_templates_expand_in_order() {',
              '        assert_eq!(GET_USERS.url(&[]), "/users");',
              '        assert_eq!(',
              '            PUT_CHANNELS_CHANNEL_ID.url(&["channel".to_owned()]),',
              '            "/channels/channel"',
              '        );',
              '        assert_eq!(',
              '            DELETE_USERS_USER_ID_ROLES_ROLE_ID',
              '                .url(&["user".to_owned(), "role".to_owned()]),',
              '            "/users/user/roles/role"',
              '        );',
              '    }',
              '}',
              '']
    return '\n'.join(lines)


def backend_spec(backend):
    backend = backend.resolve()
    try:
        root = subprocess.run(['git', '-C', str(backend), 'rev-parse', '--show-toplevel'],
                              text=True, capture_output=True, check=True).stdout.strip()
        head = subprocess.run(['git', '-C', str(backend), 'rev-parse', 'HEAD'],
                              text=True, capture_output=True, check=True).stdout.strip()
        pin = (API / 'protocol/backend-commit').read_text(encoding='utf-8').strip()
        if pathlib.Path(root).resolve() != backend:
            raise ValueError('Backend path must be the checkout root')
        if head != pin:
            raise ValueError(f'Backend HEAD {head} does not match pinned commit {pin}')
        return yaml.safe_load((backend / 'openapi.yml').read_text(encoding='utf-8'))
    except (OSError, subprocess.CalledProcessError, yaml.YAMLError) as error:
        raise ValueError(f'Cannot read backend checkout: {error}') from error


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--backend', required=True, type=pathlib.Path, help='backend checkout root')
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    try:
        output = generate(backend_spec(args.backend))
    except (ValueError, KeyError, TypeError) as error:
        raise SystemExit(f'Invalid backend contract: {error}') from error
    failed = False
    wire = API / 'src/wire'
    for path in sorted(set(wire.rglob('*.rs')) - output.keys()):
        print(f'Stale generated wire file: {path}')
        failed = True
    for path, content in output.items():
        content = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'], input=content,
                                 text=True, capture_output=True, check=True).stdout
        if args.write:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding='utf-8')
        elif not path.exists() or path.read_text(encoding='utf-8') != content:
            print(f'Protocol drift: {path}')
            failed = True
    if failed:
        raise SystemExit('Review wire changes and regenerate deliberately with --write; remove stale files manually.')


if __name__ == '__main__':
    main()
