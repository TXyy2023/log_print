# Python SDK acceptance contract

The CI workflow is defined before implementation. Every platform (Linux, macOS,
Windows) runs Python 3.12 and 3.14, builds the real Rust workspace, builds a wheel,
and installs it into a clean venv. Tests run outside the checkout using that wheel,
not a source-tree import. No PyPI publication is involved.

Required behavior:

- `async with init()` starts an owned Core when no managed environment exists;
  `send(str | bytes)` and `async for record in records()` work with defaults.
- Managed Input, Output and Transform use the supervisor environment; no Core
  is launched or terminated by a managed session. Existing asyncio loops work.
- Rust Input -> Python Output and Python Input -> Rust Output preserve bytes;
  derived records preserve upstream identity. Complete Record fields survive,
  including NUL, non-UTF-8 and integers greater than 2**53.
- Cancellation, malformed/truncated/oversized frames, wrong credentials, timeout,
  unexpected disconnect and shutdown have explicit outcomes. Side effects are
  never retried; cancellation does not create partial TCP frames.
- Bounded queues do not prevent control replies. Slow consumers cannot imply
  full delivery. Duplicate subscriptions fail; closing permits resubscription.
- Config remains static; config.patch rejects changes; shutdown acknowledges
  stopping, never fabricated completion. Failed work reports failure.
- Owned Core and subscription tasks are cleaned up after success, failure and
  initialization errors. Paths with spaces and missing binaries are covered.
- Public examples and type information are shipped alongside local installation
  instructions. CI logs contain synthetic data only, never runtime credentials.

Run `python quality/tests/python_sdk.py` after `cargo build --locked --workspace`.
The exact same runner is used locally and in GitHub Actions. Passing a local run
proves only that host; the workflow matrix establishes cross-platform results.
