# Performance

deskhaven is built for one node with 10 000 online devices. This page shows how it does
against that target on SQLite and on PostgreSQL, and how to repeat the measurement.

Both runs pass every requirement, without tuning.

## The setup

| | |
| --- | --- |
| CPU | AMD EPYC 7C13 64-Core Processor, 64 logical cores |
| Memory / OS | 47 GiB / Linux 6.18.59 |
| Build | `--release` (the profile in `Cargo.toml`: thin LTO, one codegen unit), console embedded; the load generator too. The server is built in the same `cargo build` as the load generator, so dependency features are merged across both and the binary is not byte-identical to the release image (same code, possibly a few more dependency features). |
| Server settings | defaults (`log.level` info, `database.max_connections` 10, `devices.last_seen_flush` 60 s) |
| PostgreSQL | 18.6 on the same host, default settings (`max_connections` 100, `shared_buffers` 128 MB, `fsync` on) |
| Load | 10 000 devices; phases idle 10 s, ramp 60 s, settle 30 s, steady 180 s (the steady window spans three last-seen flushes), then a restart of the server on the populated database and 10 s of idle sampling |

The load generator ran on the same machine. CPU and memory figures are those of the
deskhaven process alone. Verdicts are judged on the steady window, except idle RSS and
shutdown.

## SQLite report

```sh
ulimit -n 65536
cargo xtask loadgen --work-dir target/m404-sqlite-10k-v2 --out target/m404-sqlite-10k-v2.md
```

| Requirement | Target | Measured | Result |
| --- | --- | --- | --- |
| Heartbeat load | ≈ 667 req/s (10 000 devices / 15 s), at least 95% | 670.0 req/s | PASS |
| RegisterPeer load | ≈ 833 req/s (10 000 devices / 12 s), at least 95% | 833.3 req/s | PASS |
| Heartbeat latency, p99 | < 50 ms | 0.47 ms | PASS |
| Heartbeat failures | 0 | 0 of 120 602 | PASS |
| Registration failures (RegisterPeer, RegisterPk) | 0 | 0 of 150 000 | PASS |
| Sysinfo and audit failures | 0 | 0 of 1 994 | PASS |
| Server CPU, average | < 1 core | 0.15 cores | PASS |
| Idle RSS, restarted on the populated database | < 64 MiB | 42.3 MiB | PASS |
| Heartbeats not written row by row | `devices` row updates ≤ devices × (⌈window / flush⌉ + 1) = 50 000 | 30 000 updates for 120 602 heartbeats | PASS |
| Database writes, all tables | ≤ 50 000 + 2 per audit call + 1 per sysinfo call = 53 514 rows | 33 040 rows | PASS |
| Server shutdown | status 0 after SIGTERM, both runs | clean | PASS |

Also measured: idle RSS on the empty database before the load 33.5 MiB; heartbeat p50
0.33 ms, max 3.02 ms; busiest 10 s of CPU 0.16 cores; largest RSS under load 93.2 MiB.
Rows written in the steady window: `devices` 30 000 updated, `connection_sessions` 504
inserted and 1 016 updated, `audit_nonces` 1 520 inserted.

## PostgreSQL report

```sh
ulimit -n 65536
cargo xtask loadgen --work-dir target/m404-pg-10k-v2 \
  --database-url postgres://deskhaven@127.0.0.1:55404/deskhaven_load2 \
  --out target/m404-pg-10k-v2.md
```

| Requirement | Target | Measured | Result |
| --- | --- | --- | --- |
| Heartbeat load | ≈ 667 req/s (10 000 devices / 15 s), at least 95% | 670.0 req/s | PASS |
| RegisterPeer load | ≈ 833 req/s (10 000 devices / 12 s), at least 95% | 833.3 req/s | PASS |
| Heartbeat latency, p99 | < 50 ms | 0.47 ms | PASS |
| Heartbeat failures | 0 | 0 of 120 601 | PASS |
| Registration failures (RegisterPeer, RegisterPk) | 0 | 0 of 150 000 | PASS |
| Sysinfo and audit failures | 0 | 0 of 1 995 | PASS |
| Server CPU, average | < 1 core | 0.14 cores | PASS |
| Idle RSS, restarted on the populated database | < 64 MiB | 35.3 MiB | PASS |
| Heartbeats not written row by row | `devices` row updates ≤ devices × (⌈window / flush⌉ + 1) = 50 000 | 30 000 updates for 120 601 heartbeats | PASS |
| Database writes, all tables | ≤ 53 516 rows; ≤ 6 185 commits (flush statements + 3 per audit / sysinfo call + 100) | 33 046 rows; 2 093 commits | PASS |
| Server shutdown | status 0 after SIGTERM, both runs | clean | PASS |

Also measured: idle RSS on the empty database before the load 30.7 MiB; heartbeat p50
0.33 ms, max 5.22 ms; busiest 10 s of CPU 0.15 cores; largest RSS under load 57.9 MiB.
Rows written in the steady window: `devices` 30 000 updated, `connection_sessions` 505
inserted and 1 018 updated, `audit_nonces` 1 523 inserted.

## What the numbers say

- **Heartbeats cost no database work.** The 120 601 heartbeats of the PostgreSQL steady
  window came with 2 093 committed transactions: the audit records, the sysinfo uploads and
  three last-seen flushes of 20 statements each (500 rows per statement). Each flush wrote
  every device once: 30 000 row updates for 10 000 devices over three flushes, on both
  databases. All other rows written in the window are the audit records' (two per call).
- **Idle RSS means a server that knows its fleet.** The verdict is taken after the load:
  the server is stopped, started again on the same database (10 000 registered devices,
  which the signal server loads at startup) and sampled for 10 s without load. That is
  42.3 MiB on SQLite and 35.3 MiB on PostgreSQL, against 33.5 and 30.7 MiB for a fresh,
  empty database.
- **Memory grows with the online fleet**, since every online device is kept in memory:
  under load the largest RSS was 93.2 MiB on SQLite (its page cache lives in the process)
  and 57.9 MiB on PostgreSQL. The 64 MiB target is for an idle server.
- **First registration is the busiest moment.** While the 10 000 devices registered and
  uploaded their first sysinfo, the slowest sysinfo upload took 59 ms on SQLite (writes
  queue on its single writer) and 18 ms on PostgreSQL; heartbeats stayed below 9 ms.

## How it is measured

`cargo xtask loadgen` builds the server and the load generator
(`crates/deskhaven-loadgen`) in release mode, starts `deskhaven serve` in a new work
directory and simulates RustDesk devices the way the official client (1.4) behaves:

- **Registration**: each device sends `RegisterPk` once (a fresh install), then
  `RegisterPeer` every 12 s over UDP, and registers its key again when the server asks.
  Registration runs in its own task, as in the client, so a slow API never delays it.
- **Client API**: a 3 s timer. The first tick uploads sysinfo; a heartbeat goes out every
  15 s, or on every tick while a connection is open. Each call opens a new TCP connection,
  as the client does.
- **Occasional calls**: on average one incoming connection per device per hour (three
  `POST /api/audit/conn` records each) and one sysinfo re-upload per device per hour.
- Each device has its own loopback address (`127.64.0.1` and up), so the server's per-IP
  limits see 10 000 clients.

After the steady window the generator stops the server, starts it again on the same
database and samples it for 10 s without load.

What is recorded:

- **Latency**, by the simulated client: from opening the TCP connection to the end of the
  reply.
- **Load reached**: heartbeats and RegisterPeers per second in the steady window must be
  at least 95% of the nominal rate (devices / interval), otherwise the run does not count.
- **Failures**: any failed heartbeat, registration (UDP timeout or refusal), sysinfo or
  audit call in the steady window fails the run. On loopback nothing is lost on the way,
  so every failure is the server's.
- **CPU and RSS** from `/proc/<pid>/stat` and `/proc/<pid>/status` of the deskhaven
  process, once a second. The idle RSS verdict is the largest value after the restart on
  the populated database; the value of the empty database before the load is shown too.
- **Database writes**: rows inserted, updated and deleted per table. PostgreSQL reports
  them in `pg_stat_user_tables`, and committed transactions in `pg_stat_database`; its
  counters can trail by a few seconds, which the bounds allow for. SQLite keeps no such
  statistics, so before the load starts the generator adds triggers to every table that
  count each written row in a side table, whatever code path writes it. Each counted row
  costs one more update in the same transaction, so the SQLite figures are slightly
  pessimistic.
- **Not written row by row**: a flush writes each device at most once, so a window of *W*
  seconds allows at most *devices × (⌈W / flush⌉ + 1)* row updates of `devices`, while
  writing every heartbeat would take one per heartbeat. Over all tables, the audit and
  sysinfo calls may add two and one rows each; on PostgreSQL the commits are bounded too
  (flush statements, three per audit or sysinfo call, 100 more). A heartbeat that wrote any
  table, or committed anything, every time would exceed the bounds. The checks only count
  when the bounds are below the number of heartbeats, that is with a flush interval longer
  than the heartbeat interval (60 s against 15 s by default).
- **Shutdown**: both server processes must exit with status 0 after SIGTERM.

The generator prints a Markdown report with these verdicts, the requests per phase, the
process figures and the row writes per table. It exits with status 0 when every
requirement passes, 2 when one fails, 1 on an error. Interrupted (Ctrl-C, SIGTERM), it
stops the server before it exits. If the open-file soft limit is too low for the device
count (two per device plus 256) it raises it up to the hard limit, or stops before
starting anything when even that is too low.

## Repeat the measurement

Linux only (`/proc`, and 127.0.0.0/8 routed to the loopback interface). From the
repository root, in `nix develop`:

```sh
# 10 000 devices need about 20 000 file descriptors (the generator raises the soft
# limit up to the hard one by itself; this makes sure the hard one is high enough).
ulimit -n 65536

# SQLite, in the work directory
cargo xtask loadgen --work-dir target/loadgen-sqlite --out target/loadgen-sqlite.md

# PostgreSQL, on an empty database
initdb -D target/pg -U deskhaven --auth=trust
pg_ctl -D target/pg -o "-p 55404 -k $PWD/target -c listen_addresses=127.0.0.1" \
  -l target/pg.log -w start
createdb -h 127.0.0.1 -p 55404 -U deskhaven deskhaven_load
cargo xtask loadgen --work-dir target/loadgen-pg \
  --database-url postgres://deskhaven@127.0.0.1:55404/deskhaven_load \
  --out target/loadgen-pg.md
pg_ctl -D target/pg stop
```

The work directory must be new or empty, and the database empty. The client API and
signal ports default to 31114 and 31116 (`--api-port`, `--signal-port`), so a deskhaven on
the standard ports can keep running. `target/release/deskhaven-loadgen --help` lists every
option: device count, phase lengths, client timers, flush interval, call rates.

## Not covered

- Loopback only: no network latency, no TLS, no reverse proxy.
- One client version, no strategies assigned, no remote sessions beyond their audit
  records.
- The relay (hbbr) is not part of the run.
