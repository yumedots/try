---
name: live-ui-verification
description: Verify UI changes by driving the real running app with synthesized input and reading logs plus screenshots. Use when a UI change must be proven live, only after unit tests pass.
---

# Live UI verification

Unit tests prove the logic; they don't prove the UI. UI changes must be verified by driving the real running app with synthesized input and reading real output — logs plus screenshots or screen reads. A green compile or code review alone is not enough for UI-affecting changes.

## 1. Build and run unit tests first

Only move to live verification once unit tests pass, so a live failure is known to be UI/plumbing, not logic.

## 2. Launch the app with logging

Kill any stale instance. Launch with stdout/stderr redirected to a log file. Wait for startup before sending input.

## 3. Instrument temporarily if needed

If a value isn't otherwise observable, add a temporary debug print on that code path, rebuild, run the gesture, read the log, then remove the print and rebuild. Confirm no leftover instrumentation before reporting done. Permanent user-facing debug logs are exempt.

## 4. Navigate with synthesized input

Use the input-synthesis mechanism native to the target platform rather than forcing one from elsewhere.

## 5. Find target window or element geometry

Query on-screen bounds with the platform's native tool before computing injection coordinates. Prefer coordinates the app itself logs, when available.

## 6. Inject gestures realistically

For drag/rotate gestures: move to start, press down, several small incremental steps, release. Post to every input layer the platform requires — some platforms silently drop events posted to only one layer. Small sleeps between steps let the app's frame loop observe intermediate state.

## 7. Focus discipline

Synthesized input lands on whatever has focus or is under the cursor. Re-assert focus immediately before each gesture, since it can shift between steps. If a gesture produces zero effect, check focus first.

## 8. Capture and analyze

Screenshot or screen-read before and after the gesture to prove the change reached the pixels/DOM. Compare programmatically where possible rather than eyeballing. Treat app logs as the source of truth for exact values, screenshots as proof the redraw happened.

## 9. Cleanup and final gate

Kill the test instance, rebuild, rerun the full test suite. Before reporting done: confirm no leftover debug instrumentation, the working tree has only intended changes, and the full cycle runs to completion.
