#!/usr/bin/env python3
"""Repeat: 100 emails, 30s pause, then one email every 5s for 30s."""
import argparse
from email.message import EmailMessage
from email.utils import formatdate, make_msgid
import smtplib
import time


def send(smtp, cycle, phase, number):
    message = EmailMessage()
    message['From'] = 'load-test@example.test'
    message['To'] = 'developer@example.test'
    message['Subject'] = f'Sand Post | cycle {cycle} | {phase} | {number}'
    message['Date'] = formatdate(localtime=True)
    message['Message-ID'] = make_msgid(domain='example.test')
    message.set_content(f'Local load test: cycle {cycle}, {phase}, email {number}.')
    smtp.send_message(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', default='127.0.0.1')
    parser.add_argument('--port', type=int, default=1025)
    parser.add_argument('--cycles', type=int, default=0,
                        help='number of cycles; 0 repeats until Ctrl+C (default)')
    args = parser.parse_args()
    if args.cycles < 0:
        parser.error('--cycles must be >= 0')
    if not 1 <= args.port <= 65535:
        parser.error('--port must be between 1 and 65535')

    cycle = 0
    try:
        while args.cycles == 0 or cycle < args.cycles:
            cycle += 1
            print(f'Cycle {cycle}: burst of 100 emails', flush=True)
            with smtplib.SMTP(args.host, args.port, timeout=10) as smtp:
                for number in range(1, 101):
                    send(smtp, cycle, 'burst', number)
            print('Burst sent. Pausing for 30 seconds.', flush=True)
            time.sleep(30)
            print('Sending 6 emails, one every 5 seconds.', flush=True)
            with smtplib.SMTP(args.host, args.port, timeout=10) as smtp:
                start = time.monotonic()
                for number in range(1, 7):
                    send(smtp, cycle, 'slow', number)
                    print(f'  Slow email {number}/6 sent', flush=True)
                    time.sleep(max(0, start + number * 5 - time.monotonic()))
            print(f'Cycle {cycle} complete: 106 emails sent.', flush=True)
    except KeyboardInterrupt:
        print('\nStopped.', flush=True)
    except (OSError, smtplib.SMTPException) as error:
        parser.exit(1, f'SMTP error: {error}\n')


if __name__ == '__main__':
    main()
