"""End-to-end smoke test of a built workbench. Uses only simulated hardware.

Requires Python 3 and websockets (pip install websockets).
Starts an isolated daemon, exercises CLI, WebSocket fan-out, MCP and disk history,
then stops only the child process it created. Does not open physical serial ports.
"""
import argparse
import asyncio
import json
import pathlib
import queue
import socket
import subprocess
import tempfile
import threading
import time
import urllib.request
import websockets


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('binary', type=pathlib.Path)
    args = parser.parse_args()
    binary = str(args.binary.resolve())
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    base = f'http://127.0.0.1:{port}'
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def api(path, body=None):
        request = urllib.request.Request(base + '/api' + path, data=None if body is None else json.dumps(body).encode(), headers={'Content-Type': 'application/json', 'X-Serial-Workbench': '1'})
        with http.open(request, timeout=5) as response:
            return json.load(response)

    def cli(*args, expected=0):
        process = subprocess.run([binary, '--url', base, *args], capture_output=True, timeout=10)
        assert process.returncode == expected, process.stderr.decode(errors='replace')
        return json.loads(process.stdout)

    with tempfile.TemporaryDirectory(prefix='serial-workbench-smoke-') as directory:
        log = open(pathlib.Path(directory) / 'daemon.log', 'wb')
        daemon = subprocess.Popen([binary, 'serve', '--port', str(port), '--data-dir', directory], stdout=log, stderr=log)
        mcp = None
        try:
            for _ in range(100):
                try:
                    api('/status')
                    break
                except (OSError, ValueError):
                    if daemon.poll() is not None:
                        raise RuntimeError('daemon failed to start')
                    time.sleep(.05)
            else:
                raise RuntimeError('daemon startup timed out')
            for path in ['/', '/app.js', '/style.css']:
                with http.open(base + path) as response:
                    assert response.status == 200
                    assert len(response.read()) > 100
            opened = cli('open', '--demo')
            sid = opened['id']
            assert cli('sessions')[0]['id'] == sid
            task = cli('send', sid, '--data', 'cli-probe', '--newline', 'crlf', '--wait-for', 'OK', '--wait')
            assert task['status'] == 'succeeded'

            script = {'name': 'quick-script', 'source': 'human', 'pack': {'schema_version': '0.3', 'name': 'workbench-quick-scripts', 'macros': [{'name': 'quick-script', 'steps': [{'type': 'send', 'data': 'script-first\r\n', 'encoding': 'utf8'}, {'type': 'delay', 'ms': 20}, {'type': 'send', 'data': 'script-second\r\n', 'encoding': 'utf8'}]}]}}
            queued = api(f'/sessions/{sid}/macro', script)
            assert cli('task', queued['id'], '--wait')['status'] == 'completed'
            script_events = api(f'/sessions/{sid}/events?after=0&limit=1000')['events']
            script_tx = [e['text'] for e in script_events if e['direction'] == 'tx' and e['task_id'] == queued['id']]
            assert script_tx == ['script-first\r\n', 'script-second\r\n']

            async def shared_streams():
                url = f'ws://127.0.0.1:{port}/api/sessions/{sid}/stream?after=0'
                async with websockets.connect(url, origin=base) as a, websockets.connect(url, origin=base) as b:
                    first = json.loads(await asyncio.wait_for(a.recv(), 3))
                    second = json.loads(await asyncio.wait_for(b.recv(), 3))
                    assert first['events'][0] == second['events'][0]
                    assert any('cli-probe' in e['text'] for e in first['events'])
                    cursor = first['next_cursor']
                async with websockets.connect(f'ws://127.0.0.1:{port}/api/sessions/{sid}/stream?after={cursor}', origin=base) as resumed:
                    page = json.loads(await asyncio.wait_for(resumed.recv(), 3))
                    assert all(e['seq'] > cursor for e in page['events'])
                    assert not page['gap']
            asyncio.run(shared_streams())

            mcp = subprocess.Popen([binary, '--url', base, 'mcp'], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            replies = queue.Queue()
            def read_mcp():
                for line in mcp.stdout:
                    try:
                        replies.put(json.loads(line))
                    except ValueError:
                        replies.put({'invalid_line': line.decode(errors='replace')})
            thread = threading.Thread(target=read_mcp, daemon=True)
            thread.start()
            sequence = 0
            def rpc(method, params):
                nonlocal sequence
                sequence += 1
                request = {'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}
                mcp.stdin.write(json.dumps(request).encode() + b'\n')
                mcp.stdin.flush()
                while True:
                    response = replies.get(timeout=5)
                    if response.get('id') == sequence:
                        assert 'error' not in response, response
                        return response['result']
            rpc('initialize', {'protocolVersion': '2024-11-05', 'capabilities': {}, 'clientInfo': {'name': 'workbench-smoke', 'version': '1'}})
            mcp.stdin.write(b'{"jsonrpc":"2.0","method":"notifications/initialized"}\n')
            mcp.stdin.flush()
            tools = rpc('tools/list', {})['tools']
            assert len(tools) == 15
            def tool(name, arguments):
                result = rpc('tools/call', {'name': name, 'arguments': arguments})
                assert not result.get('isError'), result
                return json.loads(result['content'][0]['text'])
            assert tool('serial_sessions', {})[0]['id'] == sid
            task = tool('serial_send', {'session_id': sid, 'data': 'mcp-probe\r\n', 'wait_for': 'OK'})
            assert cli('task', task['id'], '--wait')['status'] == 'succeeded'
            page = tool('serial_events', {'session_id': sid, 'after': 0, 'limit': 1000})
            assert any(e['source'] == 'mcp' and e['direction'] == 'tx' for e in page['events'])
            shared = tool('serial_script_save', {'script': {'name': 'shared-status', 'commands': 'shared-one\nshared-two', 'newline': 'crlf', 'intervalMs': 20, 'waitFor': 'OK', 'timeoutMs': 2000}})
            assert any(s['id'] == shared['id'] for s in cli('scripts', 'list'))
            shared_task = cli('scripts', 'run', shared['id'], sid, '--revision', str(shared['revision']), '--wait', '--details')
            assert shared_task['status'] == 'succeeded'
            assert shared_task['script']['id'] == shared['id']
            assert shared_task['transcript']['finalized']
            assert [e['text'] for e in shared_task['transcript']['events'] if e['direction'] == 'tx'] == ['shared-one\r\n', 'shared-two\r\n']
            assert any(e['direction'] == 'rx' and 'shared-two' in e['text'] for e in shared_task['transcript']['events'])
            mcp_detail = tool('serial_task', {'task_id': shared_task['id']})
            assert len([s for s in mcp_detail['steps'] if s['kind'] == 'send']) == 2
            assert api('/tasks/' + shared_task['id'] + '/details')['command_count'] == 2
            mcp.stdin.close()
            mcp.wait(timeout=5)
            assert api('/sessions')[0]['state'] == 'connected', 'MCP detach stopped shared capture'
            timeout_task = cli('send', sid, '--data', 'timeout-probe', '--wait-for', 'NEVER_MATCH_THIS', '--timeout-ms', '100', '--wait', expected=2)
            assert timeout_task['status'] == 'failed'
            final = cli('close', sid)
            assert final['state'] == 'closed'
            disk = pathlib.Path(directory) / sid
            raw = (disk / 'rx.log').read_bytes()
            assert b'cli-probe' in raw and b'mcp-probe' in raw
            history = cli('events', sid, '--history', '--contains', 'mcp-probe')
            assert history['events']
            with http.open(base + f'/api/sessions/{sid}/export') as response:
                assert response.read() == raw
            last = cli('open', '--demo')
            last_id = last['id']
            cli('send', last_id, '--data', 'shutdown-wait', '--wait-for', 'WILL_NOT_MATCH')
            async def stop_with_live_viewer():
                url = f'ws://127.0.0.1:{port}/api/sessions/{last_id}/stream'
                async with websockets.connect(url, origin=base) as viewer:
                    await asyncio.wait_for(viewer.recv(), 3)
                    await asyncio.to_thread(cli, 'stop')
                    try:
                        while True:
                            await asyncio.wait_for(viewer.recv(), 5)
                    except websockets.exceptions.ConnectionClosed:
                        pass
            asyncio.run(stop_with_live_viewer())
            daemon.wait(timeout=5)
            assert daemon.returncode == 0
            last_info = json.loads((pathlib.Path(directory) / last_id / 'session.json').read_bytes())
            assert last_info['state'] == 'closed' and last_info['active_task'] is None
            daemon = subprocess.Popen([binary, 'serve', '--port', str(port), '--data-dir', directory], stdout=log, stderr=log)
            for _ in range(100):
                try:
                    api('/status')
                    break
                except OSError:
                    time.sleep(.05)
            assert cli('scripts', 'show', shared['id'])['commands'] == 'shared-one\nshared-two'
            recovered = cli('task', shared_task['id'], '--details')
            assert recovered['status'] == 'succeeded' and recovered['transcript']['events']
            cli('stop')
            daemon.wait(timeout=5)
            print(json.dumps({'ok': True, 'checks': ['embedded web assets', 'CLI send/expect', 'quick script ordered batch', 'WebSocket fan-out', 'cursor reconnect', 'MCP discovery and send', 'MCP detach keeps capture', 'timeout exit status', 'disk history', 'raw export', 'shared MCP/CLI/GUI scripts', 'task commands and receive window', 'persistent task and script recovery', 'graceful daemon shutdown'], 'mcp_tools': len(tools)}, indent=2))
        finally:
            if mcp and mcp.poll() is None:
                mcp.terminate()
                mcp.wait(timeout=5)
            if daemon.poll() is None:
                daemon.terminate()
                daemon.wait(timeout=5)
            log.close()


if __name__ == '__main__':
    main()
