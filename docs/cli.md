# `gantry` command line

`gantry` is the whole product without a GUI (plan §3.4). Every command
accepts `--json` and then prints one JSON document on stdout matching the
schema in [`docs/schema/`](schema/) (also printed by `gantry schema
<name>`). Diagnostics go to stderr; set `GANTRY_LOG=warn` (or `debug`) for
more.

## Global options

| Option | Environment | Default |
|---|---|---|
| `--config-dir DIR` | `GANTRY_CONFIG_DIR` | `$XDG_CONFIG_HOME/gantry` (Linux) |
| `--data-dir DIR` | `GANTRY_DATA_DIR` | `$XDG_DATA_HOME/gantry` (Linux) |
| `--json` | | human-readable text |

## Commands (M1)

| Command | Does | Schema |
|---|---|---|
| `gantry doctor` | Checks config files, data directory, database, OS keyring, browser, scheduler | `doctor` |
| `gantry run --discover-only` | One discovery run: boards, feeds, HN, probing, dedup, filters | `run` |
| `gantry postings list [--filtered-out \| --all] [--include-closed] [--limit N]` | Postings with every filter's verdict and reason | `postings-list` |
| `gantry url add URL` | Records a posting or board URL; the next run polls its board | `url-add` |
| `gantry schema [NAME]` | Prints a JSON Schema, or lists the names | none |

`gantry run` without `--discover-only` exits 4 until staging lands (M5).

## Exit codes

Stable; scripts and agents may rely on them.

| Code | Meaning |
|---|---|
| 0 | Success. A discovery run that recorded source errors still exits 0; the errors are in the report. |
| 1 | The command failed: database, file system, network setup. |
| 2 | Bad arguments or input. Nothing was changed. |
| 3 | `doctor` found at least one failing check. |
| 4 | The operation belongs to a later milestone. |
| 5 | A config file does not parse or has an unknown key. |

With `--json`, a failure prints `{"error": {"kind": ..., "message": ...}}`
(schema `error`). `kind` is one of `usage`, `invalid_input`, `config`,
`not_available`, `store`, `platform`, `network`.

## Submitting

There is no submit command yet. When it arrives (M6) it will require an
interactive terminal and a per-application code; `--json` mode will refuse
to submit (invariant 1). That guards against accidents, not against a
determined script.
