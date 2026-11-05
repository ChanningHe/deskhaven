# CLI reference

Every `deskhaven` command and option. A command that fails prints `error: …` and exits
non-zero.

In the compose deployment, run commands inside the running container:

```sh
docker compose exec deskhaven deskhaven key show
```

## Global options

They go before or after the command and apply to every command.

| Option | Description |
| --- | --- |
| `--config <PATH>` | Configuration file. Default: `./deskhaven.toml`, then `/etc/deskhaven/deskhaven.toml`, both optional. A path given here must exist. |
| `--data-dir <PATH>` | Overrides `data_dir`. |
| `--log-level <FILTER>` | Overrides `log.level`, e.g. `info` or `warn,deskhaven=debug`. |
| `--help`, `--version` | Print help or the version. |

The keys are in the [configuration reference](configuration.md).

## Commands

### `deskhaven serve`

Runs the server. This is the default when no command is given.

### `deskhaven config check`

Validates the configuration and exits: bootstrap keys, pinned settings, the database
connection, the key files, and that the listen ports are free. Because of the ports, run it
in a new container while the server is up:

```sh
docker compose run --rm deskhaven config check
```

### `deskhaven config print`

Prints the keys that are set, with their source (file, environment variable, flag). Secrets
show as `<redacted>`.

- `--effective`: also prints the keys left at their default.

### `deskhaven config schema`

Prints the JSON Schema of `deskhaven.toml`, for editors.

### `deskhaven key show`

Prints the public key on the first line: the **Key** clients use and the key the relay
needs. When `public_url` is set, the second line is the client configuration string.

### `deskhaven admin create-setup-link`

Prints a new one-time link to create the owner account. Works while the server runs, and
only while there is no owner.

### `deskhaven user unlock <USERNAME>`

Clears the sign-in lockout and the failure counter of an account. `<USERNAME>` is matched
without regard to case. Works while the server runs.

### `deskhaven healthcheck`

Exits 0 when the local server answers `/readyz` with 200, else non-zero. The compose file
uses it as the container health check.
