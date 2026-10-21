# Sandpost test runner

Send test emails to an existing SMTP server with `mail-burst`, or verify a real
Sandpost binary with `smoke-test`. Run the commands below from the repository root.

## Send test emails

With Sandpost already running, send one plain-text email:

```sh
cargo run --locked -p sandpost-test-runner -- mail-burst full --count 1
```

The default SMTP server is `127.0.0.1:1025` (`--host` and `--port` override it).
The sender is always `mail-burst@sandpost.test`; the recipient defaults to
`developer@sandpost.test`. Use `--to` to choose one recipient, including a Persona:

```sh
cargo run --locked -p sandpost-test-runner -- \
  mail-burst full --count 1 --format newsletter --to persona@sandpost.test
```

| `--format` | Content | Files in [templates/](templates/) |
| --- | --- | --- |
| `txt` (default) | Plain text | `plain.txt` |
| `html` | HTML delivery report with a text alternative | `delivery-report.html`, `delivery-report.txt` |
| `newsletter` | HTML newsletter with a text alternative and unsubscribe link | `newsletter.html`, `newsletter.txt` |

Templates support `{{cycle}}`, `{{group}}`, `{{number}}` and `{{identifier}}`.
They are embedded at compile time; `cargo run` rebuilds them after edits.
Newsletter URLs use the reserved `example.test` domain as fictional test links.

For consecutive emails, increase `full --count` (for example, `50k`). The runner
reports accepted counts and throughput; failed sends are not retried.
See all options with `mail-burst --help`.

## Burst patterns

Use `tiny --pattern` to send groups with pauses. A pattern is a comma-separated
sequence of quantities, each optionally followed by a pause. Always quote it:

```sh
cargo run --locked -p sandpost-test-runner -- \
  mail-burst tiny --pattern '5,10,2' --interval 3s --cycles 2
```

This sends 5 emails, waits 3 seconds, sends 10, waits 3 seconds, then sends 2.
It waits another 3 seconds before repeating the entire sequence once: 34 emails
across two cycles, with no pause after the last email.

| Pattern element | Meaning |
| --- | --- |
| `5` | Send 5 emails consecutively. Quantities must be positive integers. |
| `2k` or `2K` | Send 2,000 emails. |
| `rand(10,100)` | Choose a fresh random quantity from 10 through 100, inclusive, each time this group runs. Bounds also accept `k`/`K`. |
| `10s` after a quantity | Wait 10 seconds after that group. Durations accept nonnegative integers with `ms`, `s` or `m`, including `0ms`. |

- `--interval` defaults to `1s` and supplies the pause after groups without an
  explicit pause, including between cycles.
- An explicit pause **replaces** `--interval` for that group; the two are not added.
- `--cycles` defaults to `1`; `--cycles 0` repeats until Ctrl+C.
- No pause follows the final group of the final finite cycle, even if explicitly written.
- `--pattern`, `--interval` and `--cycles` belong to `tiny`; `--count` belongs to `full`.

For example, combine fixed quantities, custom pauses and random quantities:

```sh
cargo run --locked -p sandpost-test-runner -- \
  mail-burst tiny --pattern '50,10s,10,1s,rand(10,100)' --interval 2s --cycles 2
```

Each cycle sends 50 emails, waits 10 seconds, sends 10, waits 1 second, then sends
between 10 and 100 emails. It waits 2 seconds between the two cycles. The random
quantity is sampled again in the second cycle.

## Smoke test

Build the server, then run the isolated SMTP, HTTP, IMAP, search and restart checks:

```sh
cargo build --locked -p sandpost
cargo run --locked -p sandpost-test-runner -- smoke-test
```

Run the runner's own tests with `cargo test --locked -p sandpost-test-runner`.
