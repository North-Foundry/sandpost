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


def available_local_port():
    """Ask the operating system for an available local TCP port."""
    with socket.socket() as listening_socket:
        listening_socket.bind(('127.0.0.1', 0))
        return listening_socket.getsockname()[1]


def main():
    """Exercise the built server across SMTP ingestion, HTTP, persistence, and restart."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/debug/sandpost')
    parsed_arguments = parser.parse_args()
    server_binary = parsed_arguments.binary.resolve()
    if not server_binary.is_file():
        parser.error('build first with cargo build --workspace')
    web_port, mail_port = available_local_port(), available_local_port()
    while mail_port == web_port:
        mail_port = available_local_port()
    server_address = f'http://127.0.0.1:{web_port}'

    def fetch_response_data(resource_path):
        """Fetch and decode one JSON endpoint from the local server."""
        with urllib.request.urlopen(server_address + resource_path, timeout=3) as web_response:
            return json.load(web_response)

    with tempfile.TemporaryDirectory(prefix='sandpost-smoke-') as data_directory:
        environment_variables = dict(os.environ, SANDPOST_DATA_DIR=data_directory,
                   SANDPOST_DATABASE_PATH=str(Path(data_directory) / 'sandpost.sqlite3'),
                   SANDPOST_HTTP_LISTEN=f'127.0.0.1:{web_port}',
                   SANDPOST_SMTP_LISTEN=f'127.0.0.1:{mail_port}',
                   SANDPOST_LOG_LEVEL='info')
        environment_variables.pop('SANDPOST_MAX_SCOPE_DEPTH', None)
        server_log_path = Path(data_directory) / 'server.log'

        def start_server(server_log_file):
            """Start the server and wait until its health endpoint responds."""
            server_process = subprocess.Popen([str(server_binary)], env=environment_variables, stdout=server_log_file, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if server_process.poll() is not None:
                        raise RuntimeError(server_log_path.read_text())
                    try:
                        assert fetch_response_data('/api/v1/health')['status'] == 'ok'
                        return server_process
                    except (OSError, AssertionError):
                        time.sleep(0.05)
                raise RuntimeError('server readiness timeout: ' + server_log_path.read_text())
            except BaseException:
                stop_server(server_process)
                raise

        def stop_server(server_process):
            """Stop the server gracefully and fail if it cannot drain promptly."""
            if server_process.poll() is None:
                server_process.send_signal(signal.SIGTERM)
            try:
                exit_code = server_process.wait(timeout=9)
            except subprocess.TimeoutExpired:
                server_process.kill()
                server_process.wait()
                raise AssertionError('server did not drain on SIGTERM')
            assert exit_code == 0, server_log_path.read_text()

        with server_log_path.open('a') as server_log_file:
            server_process = start_server(server_log_file)
            event_stream_connection = http.client.HTTPConnection('127.0.0.1', web_port, timeout=3)
            try:
                event_stream_connection.request('GET', '/api/v1/events')
                event_stream_response = event_stream_connection.getresponse()
                assert event_stream_response.status == 200
                assert event_stream_response.getheader('content-type') == 'text/event-stream'

                def read_event():
                    """Read one complete server-sent event from its HTTP response."""
                    event_lines = []
                    while True:
                        event_line = event_stream_response.readline().decode().strip()
                        if not event_line:
                            return '\n'.join(event_lines)
                        event_lines.append(event_line)

                assert 'event: ready' in read_event()
                raw_message = (b'From: Dev <dev@BFLOW.DEV>\r\nTo: user@BORIS.IT\r\n'
                       b'Subject: Sand Post smoke\r\nX-App: bflow\r\nX-App: second\r\n'
                       b'Content-Type: text/plain; charset=utf-8\r\n\r\nHello from SMTP.\r\n.dot line\r\n')
                with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                    mail_server.sendmail('dev@bflow.dev', ['user@boris.it'], raw_message)
                assert 'event: message' in read_event()
                message_rows = fetch_response_data('/api/v1/messages')
                assert len(message_rows) == 1 and message_rows[0]['subject'] == 'Sand Post smoke', message_rows
                message_identifier = message_rows[0]['id']
                message_detail = fetch_response_data(f'/api/v1/messages/{message_identifier}')
                assert message_detail['facts']['from'][0]['domain'] == 'bflow.dev'
                assert message_detail['facts']['headers']['x-app'] == ['bflow', 'second']
                with urllib.request.urlopen(server_address + f'/api/v1/messages/{message_identifier}/raw') as downloaded:
                    assert downloaded.read() == raw_message
                with sqlite3.connect(Path(data_directory) / 'sandpost.sqlite3') as database_connection:
                    assert database_connection.execute('select count(*) from mail').fetchone()[0] == 1
                    assert database_connection.execute('select count(*) from mail_scope').fetchone()[0] == 1
                    assert database_connection.execute('pragma user_version').fetchone()[0] >= 1
                    assert database_connection.execute('pragma journal_mode').fetchone()[0] == 'wal'
                assert fetch_response_data(f"/api/v1/messages?before={message_rows[0]['seq']}") == []
            finally:
                event_stream_response.close() if 'event_stream_response' in locals() else None
                event_stream_connection.close()
                stop_server(server_process)
            server_process = start_server(server_log_file)
            try:
                assert fetch_response_data('/api/v1/messages')[0]['id'] == message_identifier
                with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                    mail_server.sendmail('', ['bounce@boris.it'], b'Subject: null sender\r\n\r\nbounce\r\n')
                assert len(fetch_response_data('/api/v1/messages')) == 2
            finally:
                stop_server(server_process)
    print('PASS: SMTP, null sender, normalization, materialization, API, raw MIME, SSE, WAL, pagination, restart, graceful shutdown')


if __name__ == '__main__':
    main()
