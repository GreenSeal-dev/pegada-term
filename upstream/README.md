# Upstream fix for EnergiBridge

`energibridge-saturating-sub.patch` is against `tdurieux/EnergiBridge` `main`
(2925da7). `cargo check` passes with it applied. It is not submitted; to open
the PR:

```sh
git clone https://github.com/tdurieux/EnergiBridge && cd EnergiBridge
git checkout -b fix/sleep-underflow
git apply /path/to/pegada-term/upstream/energibridge-saturating-sub.patch
git commit -am "Do not panic when a sample takes longer than the interval"
```

## Suggested PR text

**Title:** Do not panic when a sample takes longer than the interval

The sampling loop ends with

```rust
sleep(interval - time_before.elapsed().unwrap());
```

`Duration - Duration` panics on underflow, so EnergiBridge dies mid-measurement
whenever one iteration takes longer than `--interval`. That happens under heavy
load with small intervals, after `SIGSTOP`/`SIGCONT`, and after a suspend.
`SystemTime::elapsed()` also returns `Err` when the clock steps backwards, and
the `unwrap()` panics on that too.

This change sleeps for whatever is left of the interval, possibly nothing:

```rust
let elapsed = time_before.elapsed().unwrap_or_default();
sleep(interval.saturating_sub(elapsed));
```

Behaviour is unchanged when the iteration fits in the interval.

To reproduce the panic before the change:

```sh
energibridge -i 200 -- sleep 30 &
sleep 2; kill -STOP $!; sleep 1; kill -CONT $!
```
