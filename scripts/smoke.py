#!/usr/bin/env python3
"""Real binary: SMTP -> mail/outbox -> Tantivy -> authorized HTTP/SSE/IMAP -> restart."""
import argparse
import http.client
import imaplib
import json
import os
from pathlib import Path
import re
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


def assert_startup_summary(summary_text, database_path, web_port, mail_port, imap_port, startup_count):
    """Check each successful startup summary and its configured listener addresses."""
    normalized_rows = [' '.join(line.split()) for line in summary_text.splitlines()]
    assert normalized_rows.count('Ready to receive mail.') == startup_count, summary_text
    # SMTP and IMAP both report a loopback host.
    assert normalized_rows.count('Host 127.0.0.1') == 2 * startup_count, summary_text
    expected_summary_values = (
        'Sandpost', f'Port {mail_port}', f'Port {imap_port}',
        'Security none (passwords in clear text)', f'Listening 127.0.0.1:{imap_port}',
        'Encryption none', 'Auth optional (unauthenticated local mail accepted)',
        f'Listening 127.0.0.1:{mail_port}',
        f'API http://127.0.0.1:{web_port}/api/v1/messages',
        f'Listening 127.0.0.1:{web_port}',
        f'Database SQLite · {database_path}',
        'Attachments in original messages (SQLite)',
    )
    for expected_value in expected_summary_values:
        assert normalized_rows.count(expected_value) == startup_count, (expected_value, summary_text)


def assert_failed_startup(server_binary, environment_variables):
    """Require a startup error to exit unsuccessfully without announcing readiness."""
    completed_process = subprocess.run(
        [str(server_binary)], env=environment_variables, capture_output=True,
        text=True, timeout=10,
    )
    output_text = completed_process.stdout + completed_process.stderr
    assert completed_process.returncode != 0, output_text
    assert 'Ready to receive mail.' not in completed_process.stdout, output_text
    assert 'sandpost-smoke-secret-sentinel' not in output_text, output_text
    assert '\x1b' not in output_text, output_text
    return completed_process.stderr


def main():
    """Exercise the built server across SMTP ingestion, HTTP, persistence, and restart."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path(__file__).resolve().parents[1] / 'target/debug/sandpost')
    parser.add_argument('--frontend', type=Path, help='also verify a built static frontend directory')
    parsed_arguments = parser.parse_args()
    server_binary = parsed_arguments.binary.resolve()
    if not server_binary.is_file():
        parser.error('build first with cargo build --workspace')
    frontend_directory = parsed_arguments.frontend.resolve() if parsed_arguments.frontend else None
    if frontend_directory and not (frontend_directory / 'index.html').is_file():
        parser.error('--frontend must contain a built index.html')
    web_port, mail_port, imap_port = available_local_port(), available_local_port(), available_local_port()
    while len({web_port, mail_port, imap_port}) < 3:
        mail_port, imap_port = available_local_port(), available_local_port()
    server_address = f'http://127.0.0.1:{web_port}'

    session_token = None

    def authorized_request(resource_path):
        """Attach the normal session token to mail requests, leaving health/status public."""
        return urllib.request.Request(server_address + resource_path, headers={"Authorization": f"Bearer {session_token}"} if session_token else {})

    def post_json(resource_path, body):
        """Submit JSON through a normal endpoint, using the current session for protected actions."""
        headers = {"Content-Type": "application/json"}
        if session_token:
            headers["Authorization"] = f"Bearer {session_token}"
        request = urllib.request.Request(server_address + resource_path, data=json.dumps(body).encode(), headers=headers, method="POST")
        with urllib.request.urlopen(request, timeout=5) as response:
            return json.load(response)

    def fetch_response_data(resource_path):
        """Fetch and decode one JSON endpoint from the local server."""
        with urllib.request.urlopen(authorized_request(resource_path), timeout=3) as web_response:
            return json.load(web_response)

    def wait_for_messages(expected_count):
        """Poll the eventual index until its bounded batch exposes committed SQL mail."""
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            rows = fetch_response_data('/api/v1/messages')
            if len(rows) == expected_count:
                return rows
            time.sleep(0.025)
        raise AssertionError(f'index did not converge to {expected_count} messages: {rows}')

    with tempfile.TemporaryDirectory(prefix='sandpost-smoke-') as data_directory:
        environment_variables = dict(os.environ, SANDPOST_DATA_DIR=data_directory,
                   SANDPOST_DATABASE_PATH=str(Path(data_directory) / 'sandpost.sqlite3'),
                   SANDPOST_HTTP_LISTEN=f'127.0.0.1:{web_port}',
                   SANDPOST_SMTP_LISTEN=f'127.0.0.1:{mail_port}',
                   SANDPOST_IMAP_LISTEN=f'127.0.0.1:{imap_port}',
                   SANDPOST_LOG_LEVEL='info',
                   SMTP_PASSWORD='sandpost-smoke-secret-sentinel')
        environment_variables.pop('SANDPOST_MAX_SCOPE_DEPTH', None)
        environment_variables.pop('SANDPOST_SMTP_ENDPOINT', None)
        environment_variables.pop('SANDPOST_SMTP_ENDPOINTS', None)
        environment_variables.pop('SANDPOST_FRONTEND_DIR', None)
        for imap_setting in ('SANDPOST_IMAP_TLS', 'SANDPOST_IMAP_TLS_MODE', 'SANDPOST_IMAP_TLS_CERTIFICATE',
                             'SANDPOST_IMAP_TLS_PRIVATE_KEY', 'SANDPOST_IMAP_HOST', 'SANDPOST_IMAP_MAX_CONNECTIONS'):
            environment_variables.pop(imap_setting, None)
        if frontend_directory:
            environment_variables['SANDPOST_FRONTEND_DIR'] = str(frontend_directory)
        server_log_path = Path(data_directory) / 'server.log'
        server_error_path = Path(data_directory) / 'server.err'

        def start_server(server_log_file, server_error_file):
            """Start the server and wait until its health endpoint responds."""
            server_process = subprocess.Popen([str(server_binary)], env=environment_variables, stdout=server_log_file, stderr=server_error_file)
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    if server_process.poll() is not None:
                        raise RuntimeError(server_log_path.read_text() + server_error_path.read_text())
                    try:
                        assert fetch_response_data('/api/v1/health')['status'] == 'ok'
                        return server_process
                    except (OSError, AssertionError):
                        time.sleep(0.05)
                raise RuntimeError('server readiness timeout: ' + server_log_path.read_text() + server_error_path.read_text())
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

        with server_log_path.open('a') as server_log_file, server_error_path.open('a') as server_error_file:
            server_process = start_server(server_log_file, server_error_file)
            event_stream_connection = http.client.HTTPConnection('127.0.0.1', web_port, timeout=3)
            try:
                assert_startup_summary(server_log_path.read_text(), Path(data_directory) / 'sandpost.sqlite3', web_port, mail_port, imap_port, 1)
                assert '\x1b' not in server_log_path.read_text() + server_error_path.read_text()
                assert 'sandpost-smoke-secret-sentinel' not in server_log_path.read_text() + server_error_path.read_text()
                assert 'Ready to receive mail.' not in server_error_path.read_text()
                if frontend_directory:
                    expected_index = (frontend_directory / 'index.html').read_bytes()
                    for resource_path in ['/', '/mail', '/mail/00000000-0000-4000-8000-000000000001']:
                        with urllib.request.urlopen(server_address + resource_path, timeout=3) as response:
                            assert response.read() == expected_index
                            assert response.headers.get_content_type() == 'text/html'
                    asset_paths = re.findall(r'(?:src|href)="(/assets/[^\"]+)"', expected_index.decode())
                    assert asset_paths, 'frontend index must reference built assets'
                    for asset_path in asset_paths:
                        with urllib.request.urlopen(server_address + asset_path, timeout=3) as response:
                            assert response.read() == (frontend_directory / asset_path.lstrip('/')).read_bytes()
                    missing_connection = http.client.HTTPConnection('127.0.0.1', web_port, timeout=3)
                    try:
                        missing_connection.request('GET', '/api/v1/not-implemented')
                        missing_response = missing_connection.getresponse()
                        assert missing_response.status == 404
                        assert missing_response.read() != expected_index
                    finally:
                        missing_connection.close()
                assert fetch_response_data('/api/v1/instance') == {'initialized': False, 'setupKeyRequired': True}
                secret = re.search(r'Bootstrap secret:\s*(SP-[A-F0-9-]+)', server_log_path.read_text()).group(1)
                credentials = {'email': 'owner@example.test', 'password': 'smoke test owner password'}
                authorization = post_json('/api/v1/setup/authorize', {'setupKey': secret})['authorization']
                assert authorization != secret
                assert fetch_response_data('/api/v1/instance') == {'initialized': False, 'setupKeyRequired': True}
                post_json('/api/v1/setup', {'bootstrapSecret': authorization, 'name': 'Smoke Owner', **credentials})
                assert fetch_response_data('/api/v1/instance') == {'initialized': True, 'setupKeyRequired': False}
                session_token = post_json('/api/v1/login', credentials)['token']
                event_stream_connection.request('GET', '/api/v1/events', headers={'Authorization': f'Bearer {session_token}'})
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
                raw_message = (b'From: Dev <dev@SENDER.TEST>\r\nTo: user@RECIPIENT.TEST\r\n'
                       b'Subject: Sand Post smoke\r\nX-App: bravo\r\nX-App: second\r\n'
                       b'Content-Type: text/plain; charset=utf-8\r\n\r\nHello from SMTP.\r\n.dot line\r\n')
                with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                    mail_server.sendmail('dev@sender.test', ['user@recipient.test'], raw_message)
                assert 'event: message' in read_event()
                message_rows = wait_for_messages(1)
                assert len(message_rows) == 1 and message_rows[0]['subject'] == 'Sand Post smoke', message_rows
                message_identifier = message_rows[0]['id']
                message_detail = fetch_response_data(f'/api/v1/messages/{message_identifier}')
                assert message_detail['facts']['from'][0]['domain'] == 'sender.test'
                assert message_detail['facts']['headers']['x-app'] == ['bravo', 'second']
                with urllib.request.urlopen(authorized_request(f'/api/v1/messages/{message_identifier}/raw')) as downloaded:
                    captured_message = downloaded.read()
                    assert captured_message.endswith(raw_message)
                    assert captured_message.startswith(b'Return-Path: <dev@sender.test>\r\nReceived: ')
                with sqlite3.connect(Path(data_directory) / 'sandpost.sqlite3') as database_connection:
                    assert database_connection.execute('select count(*) from mail').fetchone()[0] == 1
                    assert database_connection.execute("select count(*) from sqlite_schema where name = 'mail_scope'").fetchone()[0] == 0
                    assert database_connection.execute("select count(*) from sqlite_schema where name = 'endpoints'").fetchone()[0] == 0
                    assert fetch_response_data('/api/v1/search/status')['pending_operations'] == 0
                    expected_scripts = sorted(path.name for path in (Path(__file__).resolve().parents[1] / 'crates/sandpost-storage-sqlite/migrations').glob('*.sql'))
                    applied_scripts = [row[0] for row in database_connection.execute('select migration from migrations order by migration')]
                    assert applied_scripts == expected_scripts, applied_scripts
                    assert database_connection.execute('pragma journal_mode').fetchone()[0] == 'wal'
                issued_access = post_json('/api/v1/smtp/accesses', {'name': 'Smoke access'})
                smtp_username = issued_access['access']['username']
                smtp_password = issued_access['password']
                assert issued_access['server']['listener']['bound_address'] == f'127.0.0.1:{mail_port}'
                listed_accesses = json.dumps(fetch_response_data('/api/v1/smtp/accesses'))
                assert smtp_username in listed_accesses and smtp_password not in listed_accesses
                assert 'password' not in listed_accesses
                with sqlite3.connect(Path(data_directory) / 'sandpost.sqlite3') as database_connection:
                    stored_hash = database_connection.execute('select password_hash from smtp_accesses').fetchone()[0]
                    assert stored_hash.startswith('$argon2id$') and smtp_password not in stored_hash
                with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                    try:
                        mail_server.login(smtp_username, 'not the generated password')
                        raise AssertionError('invalid SMTP credentials were accepted')
                    except smtplib.SMTPAuthenticationError as error:
                        assert error.smtp_code == 535
                assert fetch_response_data(f"/api/v1/messages?before={message_rows[0]['seq']}") == []
                query = urllib.parse.urlencode({'q': 'subject == "Sand Post smoke"'})
                assert fetch_response_data('/api/v1/search?' + query)[0]['id'] == message_identifier
                assert post_json('/api/v1/search/rebuild', {})['pending_operations'] == 0
                assert fetch_response_data('/api/v1/messages')[0]['id'] == message_identifier
                status = subprocess.run([str(server_binary), 'search', 'status'], env=environment_variables, capture_output=True, text=True, timeout=10)
                assert status.returncode == 0 and 'pending=0' in status.stdout, status.stderr
                locked = subprocess.run([str(server_binary), 'search', 'rebuild'], env=environment_variables, capture_output=True, text=True, timeout=10)
                assert locked.returncode != 0 and 'search writer is already running' in locked.stderr, locked.stderr

                # IMAP through Python's independent imaplib client: account issuing, LIST, SELECT,
                # FETCH, SEARCH, STORE, IDLE push of new SMTP mail, MOVE, and secret hygiene.
                issued_imap = post_json('/api/v1/imap/accounts', {'name': 'Smoke IMAP', 'mode': 'independent'})
                imap_username, imap_password = issued_imap['account']['username'], issued_imap['password']
                assert issued_imap['server'] == {'enabled': True, 'host': '127.0.0.1', 'port': imap_port,
                                                 'security': 'none', 'listen_address': f'127.0.0.1:{imap_port}'}
                listed_imap = json.dumps(fetch_response_data('/api/v1/imap/accounts'))
                assert imap_username in listed_imap and imap_password not in listed_imap and 'password' not in listed_imap
                with sqlite3.connect(Path(data_directory) / 'sandpost.sqlite3') as database_connection:
                    imap_hash = database_connection.execute('select password_hash from imap_accounts').fetchone()[0]
                    assert imap_hash.startswith('$argon2id$') and imap_password not in imap_hash
                imap_client = imaplib.IMAP4('127.0.0.1', imap_port, timeout=5)
                try:
                    for capability in ('IMAP4REV1', 'IDLE', 'UIDPLUS', 'MOVE', 'UNSELECT', 'AUTH=PLAIN'):
                        assert capability in imap_client.capabilities, imap_client.capabilities
                    assert 'IMAP4REV2' not in imap_client.capabilities
                    try:
                        imap_client.login(imap_username, 'not the generated password')
                        raise AssertionError('invalid IMAP credentials were accepted')
                    except imaplib.IMAP4.error as error:
                        assert 'AUTHENTICATIONFAILED' in str(error), error
                    imap_client.login(imap_username, imap_password)
                    status, mailboxes = imap_client.list()
                    assert status == 'OK' and any(b'INBOX' in line for line in mailboxes), mailboxes
                    assert any(b'\\Trash' in line and b'Trash' in line for line in mailboxes), mailboxes
                    status, existing = imap_client.select('INBOX')
                    assert status == 'OK' and existing == [b'1'], existing
                    imap_validity = imap_client.untagged_responses.get('UIDVALIDITY', [None])[0]
                    assert imap_validity, imap_client.untagged_responses
                    status, fetched = imap_client.uid('FETCH', '1:*', '(UID RFC822.SIZE BODY.PEEK[HEADER.FIELDS (SUBJECT)])')
                    assert status == 'OK' and b'Subject: Sand Post smoke' in fetched[0][1], fetched
                    assert f'RFC822.SIZE {len(captured_message)}'.encode() in fetched[0][0], fetched
                    status, found = imap_client.uid('SEARCH', None, 'SUBJECT', '"post smoke"')
                    assert status == 'OK' and found == [b'1'], found
                    status, body = imap_client.fetch('1', '(BODY[])')
                    assert status == 'OK' and body[0][1] == captured_message
                    status, flags = imap_client.fetch('1', '(FLAGS)')
                    assert b'\\Seen' in flags[0], flags
                    status, stored = imap_client.uid('STORE', '1', '+FLAGS', '(\\Flagged)')
                    assert status == 'OK' and b'\\Flagged' in stored[0], stored
                    idle_tag = imap_client._new_tag()
                    imap_client.send(idle_tag + b' IDLE\r\n')
                    assert imap_client.readline() == b'+ idling\r\n'
                    with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                        mail_server.sendmail('dev@sender.test', ['user@recipient.test'], b'Subject: pushed over IDLE\r\n\r\nidle\r\n')
                    assert imap_client.readline() == b'* 2 EXISTS\r\n'
                    imap_client.send(b'DONE\r\n')
                    idle_completion = imap_client.readline()
                    while not idle_completion.startswith(idle_tag):
                        assert idle_completion.startswith(b'* '), idle_completion
                        idle_completion = imap_client.readline()
                    assert idle_completion.startswith(idle_tag + b' OK'), idle_completion
                    status, moved = imap_client.uid('MOVE', '2', 'Trash')
                    assert status == 'OK', moved
                    status, trash = imap_client.status('Trash', '(MESSAGES UIDNEXT)')
                    assert status == 'OK' and b'MESSAGES 1' in trash[0], trash
                    imap_client.logout()
                except BaseException:
                    imap_client.shutdown()
                    raise
                assert len(wait_for_messages(2)) == 2
                assert imap_password not in server_log_path.read_text() + server_error_path.read_text()

            finally:
                event_stream_response.close() if 'event_stream_response' in locals() else None
                event_stream_connection.close()
                stop_server(server_process)
            server_process = start_server(server_log_file, server_error_file)
            try:
                assert_startup_summary(server_log_path.read_text(), Path(data_directory) / 'sandpost.sqlite3', web_port, mail_port, imap_port, 2)
                assert '\x1b' not in server_log_path.read_text() + server_error_path.read_text()
                assert 'sandpost-smoke-secret-sentinel' not in server_log_path.read_text() + server_error_path.read_text()
                assert smtp_password not in server_log_path.read_text() + server_error_path.read_text()
                assert 'Ready to receive mail.' not in server_error_path.read_text()
                assert fetch_response_data('/api/v1/instance') == {'initialized': True, 'setupKeyRequired': False}
                assert server_log_path.read_text().count('Bootstrap secret:') == 1
                session_token = post_json('/api/v1/login', credentials)['token']
                assert message_identifier in [row['id'] for row in fetch_response_data('/api/v1/messages')]
                with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                    mail_server.sendmail('', ['bounce@recipient.test'], b'Subject: null sender\r\n\r\nbounce\r\n')
                assert len(wait_for_messages(3)) == 3
                # SMTP access credentials survive the restart and authenticate through smtplib.
                with smtplib.SMTP('127.0.0.1', mail_port, timeout=3) as mail_server:
                    mail_server.login(smtp_username, smtp_password)
                    mail_server.sendmail('app@sender.test', ['user@recipient.test'], b'Subject: authenticated\r\n\r\nvia SMTP access\r\n')
                assert len(wait_for_messages(4)) == 4
                # IMAP identities persist: same UIDVALIDITY, existing UIDs kept, new mail appended,
                # and the moved message stays out of INBOX but in Trash.
                imap_client = imaplib.IMAP4('127.0.0.1', imap_port, timeout=5)
                try:
                    imap_client.login(imap_username, imap_password)
                    status, existing = imap_client.select('INBOX', readonly=True)
                    assert status == 'OK' and existing == [b'3'], existing
                    assert imap_client.untagged_responses.get('UIDVALIDITY', [None])[0] == imap_validity
                    status, uids = imap_client.uid('SEARCH', None, 'ALL')
                    assert status == 'OK' and uids == [b'1 3 4'], uids
                    status, flags = imap_client.uid('FETCH', '1', '(FLAGS)')
                    assert b'\\Flagged' in flags[0] and b'\\Seen' in flags[0], flags
                    status, trash = imap_client.status('Trash', '(MESSAGES)')
                    assert b'MESSAGES 1' in trash[0], trash
                    imap_client.logout()
                except BaseException:
                    imap_client.shutdown()
                    raise
            finally:
                stop_server(server_process)

        rebuilt = subprocess.run([str(server_binary), 'search', 'rebuild'], env=environment_variables, capture_output=True, text=True, timeout=30)
        assert rebuilt.returncode == 0 and 'pending=0' in rebuilt.stdout, rebuilt.stderr
        assert 'Bootstrap secret:' not in rebuilt.stdout
        with sqlite3.connect(Path(data_directory) / 'sandpost.sqlite3') as database_connection:
            assert database_connection.execute('select count(*) from mail').fetchone()[0] == 4
            assert database_connection.execute('select last_used_at from imap_accounts').fetchone()[0] is not None
            assert database_connection.execute('select raw_message from mail where identifier=?', (message_identifier,)).fetchone()[0] == captured_message
            assert database_connection.execute('select last_used_at from smtp_accesses').fetchone()[0] is not None

        assert 'shutting down listeners' in server_error_path.read_text()
        assert 'shutting down listeners' not in server_log_path.read_text()
        assert '\x1b' not in server_log_path.read_text() + server_error_path.read_text()

        occupied_mail_socket = socket.socket()
        occupied_mail_socket.bind(('127.0.0.1', 0))
        occupied_mail_socket.listen()
        try:
            failed_environment = dict(
                environment_variables,
                SANDPOST_SMTP_LISTEN=f'127.0.0.1:{occupied_mail_socket.getsockname()[1]}',
            )
            bind_error = assert_failed_startup(server_binary, failed_environment)
            assert 'address already in use' in bind_error.lower(), bind_error
        finally:
            occupied_mail_socket.close()

        storage_failure_path = Path(data_directory) / 'not-a-database'
        storage_failure_path.mkdir()
        assert_failed_startup(
            server_binary,
            dict(environment_variables, SANDPOST_DATABASE_PATH=str(storage_failure_path)),
        )

        port_zero_directory = Path(data_directory) / 'port-zero'
        port_zero_directory.mkdir()
        port_zero_database = port_zero_directory / 'sandpost.sqlite3'
        port_zero_environment = dict(
            environment_variables,
            SANDPOST_DATA_DIR=str(port_zero_directory),
            SANDPOST_DATABASE_PATH=str(port_zero_database),
            SANDPOST_HTTP_LISTEN='127.0.0.1:0',
            SANDPOST_SMTP_LISTEN='127.0.0.1:0',
            SANDPOST_IMAP_LISTEN='127.0.0.1:0',
            SANDPOST_LOG_LEVEL='off',
        )
        with server_log_path.open('a') as server_log_file, server_error_path.open('a') as server_error_file:
            output_offset = len(server_log_path.read_text())
            port_zero_process = subprocess.Popen(
                [str(server_binary)], env=port_zero_environment,
                stdout=server_log_file, stderr=server_error_file,
            )
            try:
                deadline = time.monotonic() + 10
                while time.monotonic() < deadline:
                    port_zero_output = server_log_path.read_text()[output_offset:]
                    if 'Ready to receive mail.' in port_zero_output:
                        break
                    if port_zero_process.poll() is not None:
                        raise RuntimeError(port_zero_output + server_error_path.read_text())
                    time.sleep(0.05)
                port_zero_output = server_log_path.read_text()[output_offset:]
                bound_ports = [int(port) for port in re.findall(r'Listening\s+127\.0\.0\.1:(\d+)', port_zero_output)]
                # Summary order: SMTP, IMAP, then HTTP.
                assert len(bound_ports) == 3 and all(bound_ports), port_zero_output
                assert_startup_summary(port_zero_output, port_zero_database, bound_ports[2], bound_ports[0], bound_ports[1], 1)
                with urllib.request.urlopen(f'http://127.0.0.1:{bound_ports[2]}/api/v1/health', timeout=3) as response:
                    assert json.load(response)['status'] == 'ok'
                with socket.create_connection(('127.0.0.1', bound_ports[0]), timeout=3) as mail_connection:
                    assert mail_connection.recv(128).startswith(b'220 ')
                with socket.create_connection(('127.0.0.1', bound_ports[1]), timeout=3) as imap_connection:
                    assert imap_connection.recv(256).startswith(b'* OK [CAPABILITY IMAP4rev1 ')
                assert '\x1b' not in port_zero_output + server_error_path.read_text()
                assert 'sandpost-smoke-secret-sentinel' not in port_zero_output + server_error_path.read_text()
                assert 'Ready to receive mail.' not in server_error_path.read_text()
            finally:
                stop_server(port_zero_process)
    print('PASS: startup summary, bind/storage failures, port 0, SMTP, normalization, API, raw MIME, SSE, search status/rebuild/locking, IMAP (imaplib, IDLE, MOVE, UID persistence), persistence, restart, shutdown' + (', static assets and SPA routing' if frontend_directory else ''))


if __name__ == '__main__':
    main()
