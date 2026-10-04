#!/usr/bin/env python3
"""Real binary: SMTP -> MIME -> matcher -> SQLite -> HTTP/SSE -> restart."""
import argparse
import http.client
import json
import os
from pathlib import Path
import signal
import smtplib
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request


def free_port():
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        return sock.getsockname()[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/debug/sandpost')
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error('build first with cargo build --workspace')
    http_port, smtp_port = free_port(), free_port()
    while smtp_port == http_port:
        smtp_port = free_port()
    base = f'http://127.0.0.1:{http_port}'

    def get(path):
        with urllib.request.urlopen(base + path, timeout=3) as response:
            return json.load(response)

    with tempfile.TemporaryDirectory(prefix='sandpost-smoke-') as data:
        env = dict(os.environ, SANDPOST_DATA_DIR=data,
                   SANDPOST_DATABASE_PATH=str(Path(data) / 'sandpost.sqlite3'),
                   SANDPOST_HTTP_LISTEN=f'127.0.0.1:{http_port}',
                   SANDPOST_SMTP_LISTEN=f'127.0.0.1:{smtp_port}',
                   SANDPOST_LOG_LEVEL='info')
        env.pop('SANDPOST_MAX_SCOPE_DEPTH', None)
        log_path = Path(data) / 'server.log'

        def start(log):
            process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if process.poll() is not None:
                        raise RuntimeError(log_path.read_text())
                    try:
                        assert get('/api/v1/health')['status'] == 'ok'
                        return process
                    except (OSError, AssertionError):
                        time.sleep(0.05)
                raise RuntimeError('server readiness timeout: ' + log_path.read_text())
            except BaseException:
                stop(process)
                raise

        def stop(process):
            if process.poll() is None:
                process.send_signal(signal.SIGTERM)
            try:
                code = process.wait(timeout=9)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                raise AssertionError('server did not drain on SIGTERM')
            assert code == 0, log_path.read_text()

        with log_path.open('a') as log:
            process = start(log)
            events = http.client.HTTPConnection('127.0.0.1', http_port, timeout=3)
            try:
                events.request('GET', '/api/v1/events')
                response = events.getresponse()
                assert response.status == 200
                assert response.getheader('content-type') == 'text/event-stream'

                def read_event():
                    lines = []
                    while True:
                        line = response.readline().decode().strip()
                        if not line:
                            return '\n'.join(lines)
                        lines.append(line)

                assert 'event: ready' in read_event()
                raw = (b'From: Dev <dev@BFLOW.DEV>\r\nTo: user@BORIS.IT\r\n'
                       b'Subject: Sand Post smoke\r\nX-App: bflow\r\nX-App: second\r\n'
                       b'Content-Type: text/plain; charset=utf-8\r\n\r\nHello from SMTP.\r\n.dot line\r\n')
                with smtplib.SMTP('127.0.0.1', smtp_port, timeout=3) as smtp:
                    smtp.sendmail('dev@bflow.dev', ['user@boris.it'], raw)
                assert 'event: message' in read_event()
                rows = get('/api/v1/messages')
                assert len(rows) == 1 and rows[0]['subject'] == 'Sand Post smoke', rows
                message_id = rows[0]['id']
                detail = get(f'/api/v1/messages/{message_id}')
                assert detail['facts']['from'][0]['domain'] == 'bflow.dev'
                assert detail['facts']['headers']['x-app'] == ['bflow', 'second']
                with urllib.request.urlopen(base + f'/api/v1/messages/{message_id}/raw') as downloaded:
                    assert downloaded.read() == raw
                with sqlite3.connect(Path(data) / 'sandpost.sqlite3') as db:
                    assert db.execute('select count(*) from messages').fetchone()[0] == 1
                    assert db.execute('select count(*) from message_scope').fetchone()[0] == 1
                    assert db.execute('pragma user_version').fetchone()[0] >= 1
                    assert db.execute('pragma journal_mode').fetchone()[0] == 'wal'
                assert get(f"/api/v1/messages?before={rows[0]['seq']}") == []
            finally:
                response.close() if 'response' in locals() else None
                events.close()
                stop(process)
            process = start(log)
            try:
                assert get('/api/v1/messages')[0]['id'] == message_id
                with smtplib.SMTP('127.0.0.1', smtp_port, timeout=3) as smtp:
                    smtp.sendmail('', ['bounce@boris.it'], b'Subject: null sender\r\n\r\nbounce\r\n')
                assert len(get('/api/v1/messages')) == 2
            finally:
                stop(process)
    print('PASS: SMTP, null sender, normalization, materialization, API, raw MIME, SSE, WAL, pagination, restart, graceful shutdown')


if __name__ == '__main__':
    main()
