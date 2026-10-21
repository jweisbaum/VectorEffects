#!/usr/bin/env python3
"""Native plugin/stdio contract. Host installation is opt-in and always uses a fresh profile."""
import argparse, json, os, pathlib, queue, shutil, subprocess, tempfile, threading, zipfile


def run(command, env, cwd):
    result = subprocess.run([str(x) for x in command], env=env, cwd=cwd, capture_output=True, text=True, timeout=90)
    if result.returncode:
        raise AssertionError(f'{command[0]} {command[1:]} failed: {result.stdout[:2000]} {result.stderr[:2000]}')
    return result.stdout


def stdio(command, env, cwd):
    process = subprocess.Popen(command, env=env, cwd=cwd, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    replies = queue.Queue()
    def read():
        for line in process.stdout:
            replies.put(json.loads(line))
    thread = threading.Thread(target=read, daemon=True); thread.start()
    def send(value):
        process.stdin.write(json.dumps(value) + '\n'); process.stdin.flush()
    def response(wanted):
        while True:
            value = replies.get(timeout=20)
            if value.get('id') == wanted:
                assert 'error' not in value, value
                return value['result']
    try:
        send({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-11-25','capabilities':{},'clientInfo':{'name':'native-contract','version':'1'}}})
        assert response(1)['capabilities']['tools'] is not None
        send({'jsonrpc':'2.0','method':'notifications/initialized'})
        send({'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}})
        tools = response(2)['tools']; assert tools
        assert len({tool['name'] for tool in tools}) == len(tools)
        for tool in tools:
            assert tool['inputSchema']['type'] == 'object'
            assert all(key in tool['annotations'] for key in ('readOnlyHint','destructiveHint','idempotentHint','openWorldHint')), tool['name']
        return len(tools)
    finally:
        process.stdin.close()
        try: process.wait(timeout=5)
        except subprocess.TimeoutExpired: process.kill(); process.wait()
        process.stdout.close(); process.stderr.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('executable', type=pathlib.Path)
    parser.add_argument('--hosts', action='store_true', help='Exercise installed Codex/Claude CLIs in isolated profiles')
    parser.add_argument('--app', action='store_true', help='Smoke-test a real app --mcp binary without launching its GUI')
    args = parser.parse_args(); executable = args.executable.resolve()
    with tempfile.TemporaryDirectory(prefix='whirlwind-mcp-contract-') as folder:
        home = pathlib.Path(folder); env = dict(os.environ)
        env.update(CODEX_HOME=str(home/'.codex'), CLAUDE_CONFIG_DIR=str(home/'.claude'))
        # No model call, credentials, network download or real host configuration is needed.
        if args.app:
            print(f'{executable.name}: {stdio([str(executable),"--mcp"],env,home)} tools over native stdio')
            return
        ids = ['vectoreffects','hindsight-sync','hindsight-router','polarexplorer']
        for plugin in ids:
            root = pathlib.Path(run([executable, home, plugin], env, home).strip())
            for manifest, variable in [('mcp.json','${PLUGIN_ROOT}'),('.mcp.json','${CLAUDE_PLUGIN_ROOT}')]:
                value = json.loads((root/manifest).read_text()); server = next(iter(value['mcpServers'].values()))
                assert 'env' not in server and 'token' not in json.dumps(value).lower()
                command = [server['command'].replace(variable,str(root)),*server['args']]
                assert stdio(command,env,home) == 1
            with zipfile.ZipFile(root/'fixture.mcpb') as archive:
                manifest = json.loads(archive.read('manifest.json'))
                entry = manifest['server']['entry_point']
                assert archive.read(entry)
                assert archive.getinfo(entry).external_attr >> 16 & 0o111
                assert manifest['server']['type'] == 'binary'
            print(f'{plugin}: portable plugin, Claude plugin, native stdio and MCPB passed')
        if args.hosts:
            for host, cli in [('codex',shutil.which('codex')),('claude_code',shutil.which('claude'))]:
                if not cli: raise AssertionError(f'{host} CLI is required for --hosts')
                for plugin in ids:
                    connect = json.loads(run([executable,home,plugin,'connect',host],env,home)); assert connect['state']=='restart_required',connect
                    status = json.loads(run([executable,home,plugin,'status',host],env,home)); assert status['state']=='restart_required',status
                    # Repeated connect is repair, not a duplicate registration or permission prompt.
                    json.loads(run([executable,home,plugin,'connect',host],env,home))
                    if host=='claude_code': run([cli,'plugin','validate',home/'.whirlwind/marketplace/plugins'/plugin],env,home)
                for plugin in ids:
                    status = json.loads(run([executable,home,plugin,'disconnect',host],env,home)); assert status['state']=='disconnected',status
                print(f'{host}: four installs, repairs, persisted states and disconnects passed in an isolated profile')

if __name__ == '__main__': main()
