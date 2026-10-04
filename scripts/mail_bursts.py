#!/usr/bin/env python3
"""Repeat: 100 emails, 30s pause, then one email every 5s for 30s."""
import argparse
from email.message import EmailMessage
from email.utils import formatdate, make_msgid
import smtplib
import time


def send_test_message(mail_server, cycle_number, phase_name, message_number):
    """Send one synthetic test message through the connected mail server."""
    message = EmailMessage()
    message['From'] = 'load-test@example.test'
    message['To'] = 'developer@example.test'
    message['Subject'] = f'Sand Post | cycle {cycle_number} | {phase_name} | {message_number}'
    message['Date'] = formatdate(localtime=True)
    message['Message-ID'] = make_msgid(domain='example.test')
    message.set_content(f'Local load test: cycle {cycle_number}, {phase_name}, email {message_number}.')
    mail_server.send_message(message)


def main():
    """Send repeated bursts and paced messages to the local SMTP listener."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', default='127.0.0.1')
    parser.add_argument('--port', type=int, default=1025)
    parser.add_argument('--cycles', type=int, default=0,
                        help='number of cycles; 0 repeats until Ctrl+C (default)')
    parsed_arguments = parser.parse_args()
    if parsed_arguments.cycles < 0:
        parser.error('--cycles must be >= 0')
    if not 1 <= parsed_arguments.port <= 65535:
        parser.error('--port must be between 1 and 65535')

    cycle_number = 0
    try:
        while parsed_arguments.cycles == 0 or cycle_number < parsed_arguments.cycles:
            cycle_number += 1
            print(f'Cycle {cycle_number}: burst of 100 emails', flush=True)
            with smtplib.SMTP(parsed_arguments.host, parsed_arguments.port, timeout=10) as mail_server:
                for message_number in range(1, 101):
                    send_test_message(mail_server, cycle_number, 'burst', message_number)
            print('Burst sent. Pausing for 30 seconds.', flush=True)
            time.sleep(30)
            print('Sending 6 emails, one every 5 seconds.', flush=True)
            with smtplib.SMTP(parsed_arguments.host, parsed_arguments.port, timeout=10) as mail_server:
                phase_started_at = time.monotonic()
                for message_number in range(1, 7):
                    send_test_message(mail_server, cycle_number, 'slow', message_number)
                    print(f'  Slow email {message_number}/6 sent', flush=True)
                    time.sleep(max(0, phase_started_at + message_number * 5 - time.monotonic()))
            print(f'Cycle {cycle_number} complete: 106 emails sent.', flush=True)
    except KeyboardInterrupt:
        print('\nStopped.', flush=True)
    except (OSError, smtplib.SMTPException) as error:
        parser.exit(1, f'SMTP error: {error}\n')


if __name__ == '__main__':
    main()
