"""Provider contract tests with real rclone transfers to disposable local storage.

The adapter reports synthetic drive/dropbox metadata, then delegates file I/O to
real rclone local remotes. This verifies Rust routing and data integrity, NOT
Google/Dropbox OAuth or live provider APIs. Never reads personal rclone config.
Use --serve PORT for an isolated browser acceptance session; Ctrl-C cleans up.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'test-output/providers'
OUT.mkdir(parents=True, exist_ok=True)


def fixture(temp):
    real = shutil.which('rclone')
    assert real, 'Install rclone before running integration tests'
    binary = temp / 'rust-word'
    shutil.copy2(ROOT / 'target/release/rust-word', binary)
    remotes = [{'name': 'fixture-google', 'type': 'drive'},
               {'name': 'fixture-dropbox', 'type': 'dropbox'},
               {'name': 'fixture-nas', 'type': 'webdav'}]
    (temp / 'remotes.json').write_text(json.dumps(remotes))
    config = temp / 'fixture.conf'
    config.write_text(''.join(f'[{r["name"]}]\ntype = alias\nremote = {temp / r["name"]}\n\n' for r in remotes))
    config.chmod(0o600)
    for remote in remotes:
        (temp / remote['name'] / 'RustWord' / 'nested').mkdir(parents=True)
    # Fixed fixture executable beside the copied app: production binaries have
    # no test backdoor or configurable executable path.
    adapter = temp / 'rclone'
    adapter.write_text('#!' + sys.executable + '\n' + '''
import json, os, pathlib, subprocess, sys
root = pathlib.Path(__file__).parent
args = sys.argv[1:]
with (root / 'events.jsonl').open('a') as out:
    out.write(json.dumps({'args': args, 'proxyPresent': bool(os.environ.get('HTTPS_PROXY')), 'noProxyPresent': bool(os.environ.get('NO_PROXY'))}) + '\\n')
if args[0] == 'listremotes':
    print((root / 'remotes.json').read_text())
    sys.exit(0)
if args[0] != 'version':
    for flag in ['--drive-export-formats', '--drive-import-formats']:
        if flag in args:
            i = args.index(flag)
            del args[i:i+2]
os.environ['RCLONE_CONFIG'] = str(root / 'fixture.conf')
sys.exit(subprocess.call(''' + repr([real]) + ''' + args))
''')
    adapter.chmod(0o700)
    # Clear inherited rclone variables, including credentials and flags. Fake
    # proxy values test inheritance; local remotes never use the network.
    env = {k: v for k, v in os.environ.items() if not k.startswith('RCLONE_')}
    env.update(RCLONE_CONFIG=str(config), HTTPS_PROXY='http://127.0.0.1:9', NO_PROXY='localhost,127.0.0.1')
    return binary, env


def main():
    checks = []
    with tempfile.TemporaryDirectory(prefix='rust-word-providers-') as name:
        temp = Path(name)
        binary, env = fixture(temp)
        if len(sys.argv) == 3 and sys.argv[1] == '--serve':
            port = int(sys.argv[2])
            print('Synthetic provider fixture at http://127.0.0.1:' + str(port), flush=True)
            process = subprocess.Popen([str(binary), '--test-server', f'127.0.0.1:{port}', str(temp / 'profile')], env=env)
            try:
                process.wait()
            except KeyboardInterrupt:
                process.terminate()
                process.wait(timeout=5)
            return
        process = subprocess.Popen([str(binary), '--rpc', str(temp / 'profile')], env=env,
                                   stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)

        def rpc(command, **args):
            process.stdin.write(json.dumps({'id': 1, 'command': command, **args}) + '\n')
            process.stdin.flush()
            return json.loads(process.stdout.readline())

        def call(command, **args):
            response = rpc(command, **args)
            assert 'error' not in response, response
            return response['data']

        def check(ok, message):
            assert ok, message
            checks.append(message)
            print('PASS ' + message, flush=True)

        try:
            discovered = call('driveDiscover')
            check({r['type'] for r in discovered['remotes']} == {'drive', 'dropbox', 'webdav'}, 'Provider metadata discovery')
            check(discovered['setupCommand'].endswith("' config"), 'Bundled executable authorization command')
            for provider, remote in [('google-drive', 'fixture-google'), ('dropbox', 'fixture-dropbox')]:
                before = len((temp / 'events.jsonl').read_text().splitlines())
                mismatch = rpc('driveConnect', provider=provider, remote='fixture-nas:', folder='must-not-exist')
                check('不匹配' in mismatch.get('error', ''), provider + ' rejects mismatched backend')
                events = [json.loads(s) for s in (temp / 'events.jsonl').read_text().splitlines()][before:]
                check(all(e['args'][0] == 'listremotes' for e in events), provider + ' rejects before remote I/O')
                connection = call('driveConnect', provider=provider, remote=remote + ':', folder='RustWord', readonly=False, createFolder=True)['connection']
                cid = connection['id']
                check(connection['provider'] == provider, provider + ' connects and persists provider')
                listing = call('driveList', connectionId=cid)
                check(listing['directories'][0]['name'] == 'nested', provider + ' browses actual directories')
                doc = {'type': 'doc', 'content': [{'type': 'paragraph', 'content': [{'type': 'text', 'text': 'Editable 中文 cloud copy'}]}]}
                export = call('exportDocument', document=doc, name='Cloud copy')
                uploaded = call('driveUpload', connectionId=cid, path='nested', **export)
                check(uploaded['readbackVerified'] and uploaded['sha256'] == hashlib.sha256(base64.b64decode(export['data'])).hexdigest(), provider + ' uploads real DOCX and verifies SHA-256 readback')
                downloaded = call('driveDownload', connectionId=cid, path='nested/' + export['name'])
                check(call('importDocument', **downloaded)['document'] == doc, provider + ' downloads editable DOCX without content loss')
                duplicate = rpc('driveUpload', connectionId=cid, path='nested', name=export['name'], data=base64.b64encode(b'wrong').decode())
                check('同名' in duplicate.get('error', ''), provider + ' rejects existing filenames')
                check(call('driveDownload', connectionId=cid, path='nested/' + export['name'])['data'] == export['data'], provider + ' preserves existing file bytes')
                readonly = call('driveConnect', provider=provider, remote=remote + ':', folder='RustWord', readonly=True, createFolder=False)['connection']
                check(call('driveList', connectionId=readonly['id'])['readonly'], provider + ' read-only browsing')
                check('只读' in rpc('driveUpload', connectionId=readonly['id'], **export).get('error', ''), provider + ' rejects read-only upload')
                call('driveDisconnect', connectionId=cid)
                check((temp / remote / 'RustWord/nested' / export['name']).exists(), provider + ' disconnect retains remote files')
            events = [json.loads(s) for s in (temp / 'events.jsonl').read_text().splitlines()]
            check(all(e['proxyPresent'] and e['noProxyPresent'] for e in events), 'Proxy and NO_PROXY environment reach rclone')
            google = [e['args'] for e in events if any(str(a).startswith('fixture-google:') for a in e['args'])]
            check(google and all('--drive-export-formats' in a and a[a.index('--drive-import-formats') + 1] == '' for a in google), 'Google Office exports with lossy upload conversion disabled')
            check(call('driveDiscover')['connections'] == [], 'Disconnect updates persistent metadata')
            (OUT / 'checks.json').write_text(json.dumps({'passed': len(checks), 'checks': checks, 'scope': 'Synthetic provider metadata and real rclone local transfers; no live OAuth/API claim'}, indent=2) + '\n')
        finally:
            process.terminate()
            process.wait(timeout=5)


if __name__ == '__main__':
    main()
