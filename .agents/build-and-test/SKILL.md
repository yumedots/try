---
name: build-and-test
description: Build, run, and test this project using its own build system. Use when compiling, launching the app or binary, running unit tests, or after source or build-definition changes.
---

# Build and verification

Use the project's own build system only. Don't introduce a second, competing build system.

Detect the build tool from what's already in the repo rather than assuming one. Build after any source or build-definition change, using that tool's standard build command. Run the test suite after any source or feature change, using that tool's standard test command.

If the project has separate app/binary vs library/test targets, build and run each explicitly — don't assume a bare "run" resolves to a default target. Never let a full app launch happen as a side effect of running tests, and never let the test suite run as a side effect of launching the app.

Match whatever language, standard, or version the project already declares rather than assuming a default. Vendored or third-party code keeps its own native language and build conventions.
