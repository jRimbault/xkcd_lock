# CLI

## Verbosity

`xkcd_lock` supports `-q` / `--quiet` and repeated `-v` / `--verbose` flags through
`clap-verbosity-flag`.

- `-q` silences logs
- default only shows errors
- `-v` shows warnings
- `-vv` shows info
- `-vvv` shows debug
- `-vvvv` shows trace

If `RUST_LOG` is set, it still overrides the default filter.

## Cache Health

Use `xkcd_lock cache health` to inspect the on-disk cache without trying to
lock the screen.

The command reports:

- the cache root
- whether the cache looks healthy overall
- the status of the cached latest-comic marker
- how many raw images, metadata files, and rendered backgrounds look valid
- leftover staged files from interrupted atomic writes

The command exits successfully when the cache is healthy and exits with an
error when it finds malformed entries or abandoned staged files.
