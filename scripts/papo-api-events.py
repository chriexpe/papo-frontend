#!/usr/bin/env python3
"""Check or generate gateway records from a checkout of the pinned backend."""
import argparse
import json
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
API = ROOT / 'crates/sdk/api'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--backend', type=pathlib.Path, required=True, help='pinned backend checkout root')
parser.add_argument('--write', action='store_true', help='rewrite generated gateway files')
args = parser.parse_args()
pin = (API / 'protocol/backend-commit').read_text(encoding='utf-8').strip()
try:
    root = subprocess.run(['git', '-C', str(args.backend), 'rev-parse', '--show-toplevel'],
                          text=True, capture_output=True, check=True).stdout.strip()
    head = subprocess.run(['git', '-C', str(args.backend), 'rev-parse', 'HEAD'],
                          text=True, capture_output=True, check=True).stdout.strip()
except (OSError, subprocess.CalledProcessError) as error:
    raise SystemExit(f'Cannot inspect backend checkout: {error}') from error
if pathlib.Path(root).resolve() != args.backend.resolve():
    raise SystemExit('Backend argument must be the checkout root')
if head != pin:
    raise SystemExit(f'Backend HEAD {head} does not match pin {pin}')

backend = args.backend / 'backend/internal'
structs = {}
for filename in ['websocket/events.go', 'websocket/presence.go', 'webrtc/events.go', 'models/voice.go']:
    source = (backend / filename).read_text(encoding='utf-8')
    for name, body in re.findall(r'type (\w+) struct \{(.*?)\n\}', source, re.S):
        fields = []
        for line in body.splitlines():
            if '`json:' not in line:
                continue
            match = re.search(r'\w+\s+(\[\][\w.]+|\*[\w.]+|[\w.]+)\s+`json:"(\w+)([^"`]*)"', line)
            if not match:
                raise SystemExit(f'Unsupported JSON field in {filename}: {line.strip()}')
            typ, field, opts = match.groups()
            if field != 'type':
                fields.append([field, typ, opts])
        structs[name] = fields
names = {}
for filename in ['websocket/events.go', 'webrtc/events.go']:
    for name, kind in re.findall(r'EventType(\w+)\s+(?:EventType\s+)?=\s+"([^"]+)"', (backend / filename).read_text(encoding='utf-8')):
        names[name] = kind
manifest = {}
for name, fields in structs.items():
    if name.endswith('Outbound'):
        short = name.removesuffix('Outbound')
        manifest[names[short]] = {'name': short + 'Event', 'fields': fields}
for name in ['VoiceJoined', 'VoiceAnswer', 'VoiceOffer', 'VoiceICECandidate', 'VoiceStateUpdate',
             'VoiceLeave', 'ActiveSpeakerUpdate', 'VoiceAudioRoutes']:
    manifest[names[name]] = {'name': name + 'Event', 'fields': structs[name]}
for name in ['PresenceMember', 'VoiceState', 'VoiceAudioRoute']:
    manifest['_' + name] = {'name': name, 'fields': structs[name]}

scalar = {'string': 'String', 'int': 'i64', 'bool': 'bool', 'time.Time': 'String',
          'models.MessageAttachment': 'crate::wire::Attachment',
          'models.ExternalAuthor': 'crate::wire::ExternalAuthor',
          'models.Embed': 'crate::wire::Embed',
          'models.DirectConversation': 'crate::wire::DirectConversation',
          'models.VoiceState': 'VoiceState', 'PresenceMember': 'PresenceMember',
          'VoiceAudioRoute': 'VoiceAudioRoute'}

def rust_type(typ):
    if typ.startswith('*'):
        return 'Option<' + rust_type(typ[1:]) + '>'
    if typ.startswith('[]'):
        return 'Vec<' + rust_type(typ[2:]) + '>'
    return scalar[typ]

files = {}
mods = []
groups = {name: {} for name in ['messages', 'channels', 'presence', 'membership', 'direct_messages', 'notifications', 'control', 'voice']}
for kind, record in manifest.items():
    if kind.startswith(('voice_', 'active_speaker_', '_Voice')):
        group = 'voice'
    elif kind.startswith(('message', 'attachment_', 'react_')):
        group = 'messages'
    elif kind.startswith('channel_'):
        group = 'channels'
    elif kind in ['user_join', 'role_add', 'role_remove']:
        group = 'membership'
    elif kind == 'dm_update':
        group = 'direct_messages'
    elif kind == 'new_notification':
        group = 'notifications'
    elif kind in ['heartbeat_ack', 'error']:
        group = 'control'
    else:
        group = 'presence'
    groups[group][kind] = record
for name, records in groups.items():
    mods.append(f'#[path = "{name}.rs"]\nmod {name};\npub use {name}::*;')
    text = '//! Generated gateway payloads; leaves transport and policy outside.\nuse super::*;\n'
    for kind, record in records.items():
        text += f'/// Backend `{kind}` payload; contains no client state.\n#[derive(Clone, Default, serde::Deserialize, serde::Serialize)]\npub struct {record["name"]} {{\n'
        for field, typ, opts in record['fields']:
            text += f'    /// Backend `{field}` field.\n'
            if typ.startswith('[]'):
                text += '    #[serde(default, deserialize_with = "crate::gateway::nullable_list")]\n'
            elif 'omitempty' in opts:
                text += '    #[serde(default)]\n'
            text += f'    pub {field}: {rust_type(typ)},\n'
        text += '}\n'
    files[API / f'src/gateway/{name}.rs'] = text
text = '//! Generated known event dispatch; unknown event preservation belongs to the gateway.\n'
text += '\n'.join(mods) + '\n'
text += '/// Every outbound event emitted by the pinned backend, including voice offers.\n#[derive(Clone, serde::Deserialize, serde::Serialize)]\n#[serde(tag = "type")]\n#[non_exhaustive]\npub enum KnownEvent {\n'
for kind, record in manifest.items():
    if kind.startswith('_'): continue
    variant = record['name'].removesuffix('Event')
    text += f'    /// Backend `{kind}` event.\n    #[serde(rename = "{kind}")]\n    {variant}(Box<{record["name"]}>),\n'
text += '}\nimpl KnownEvent {\n/// Static event kind, safe for diagnostic output.\n#[must_use]\npub const fn kind(&self) -> &\'static str { match self {\n'
for kind, record in manifest.items():
    if not kind.startswith('_'):
        text += f'Self::{record["name"].removesuffix("Event")}(_) => "{kind}",\n'
text += '} }\n/// Whether this discriminator belongs to the pinned outbound protocol.\n#[must_use]\npub fn recognizes(kind: &str) -> bool { matches!(kind, ' + ' | '.join(json.dumps(k) for k in manifest if not k.startswith('_')) + ') }\n}\n'
text += '#[cfg(test)]\nmod tests {\n//! Exercises all generated outbound variants without a backend checkout.\nuse super::KnownEvent;\n#[test]\nfn every_known_event_round_trips_through_dispatch() {\nlet events = [\n'
for kind, record in manifest.items():
    if not kind.startswith('_'):
        text += f'KnownEvent::{record["name"].removesuffix("Event")}(Box::default()),\n'
text += '];\nfor event in events {\nlet kind = event.kind();\nlet bytes = serde_json::to_vec(&event).unwrap();\nlet decoded = crate::gateway::Event::decode(&bytes).unwrap();\nassert_eq!(decoded.kind(), kind);\nassert!(matches!(decoded, crate::gateway::Event::Known(_)));\n}\n}\n}\n'
files[API / 'src/gateway/known.rs'] = text
failed = False
for path, text in files.items():
    # Records only need imports for cross-record references.
    imports = [name for name in ['PresenceMember', 'VoiceState', 'VoiceAudioRoute'] if f'Vec<{name}>' in text and f'pub struct {name} ' not in text]
    text = text.replace('use super::*;\n', ('use super::{' + ', '.join(imports) + '};\n') if imports else '')
    text = subprocess.run(['rustfmt', '--edition', '2024', '--emit', 'stdout'], input=text,
                          text=True, capture_output=True, check=True).stdout
    if args.write:
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding='utf-8')
    elif not path.exists() or path.read_text(encoding='utf-8') != text:
        print('Gateway drift:', path.relative_to(ROOT))
        failed = True
if failed:
    sys.exit(1)
