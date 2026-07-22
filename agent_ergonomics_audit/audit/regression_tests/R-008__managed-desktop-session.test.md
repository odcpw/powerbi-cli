# R-008 Managed Desktop Session

- `desktop close` is idempotent without a receipt.
- A stale PID receipt is removed without targeting a process.
- An invalid receipt is retained for explicit recovery.
- A concurrent lifecycle lock fails fast without replacing the lock.
- Live Windows smoke opens one owned session, closes it, and leaves zero Desktop processes.
