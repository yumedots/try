---
name: vendoring
description: Vendor, update, or modify vendored/third-party source kept in this project. Use when touching vendored code, syncing upstream, or changing external dependencies kept in-tree.
---

# Vendoring and external code changes

## Adoption

Identify where vendored code lives and treat it as a pinned fork: record the upstream project, version/tag, and commit hash in a README or comment at the vendor root. Preserve upstream file and symbol naming and layout unless the project has an established renaming convention. Keep every license and notice file intact. Remove non-legal source comments only when requested.

If a project layer wraps or extends vendored code, keep it clearly separated from the untouched vendored files.

## Change discipline

Before changing external code, check its license and note the exact pinned revision you're diverging from. Build and test through the host project's own build system — never introduce the vendor's original build system into the project. Don't delete generated build directories or third-party source unless explicitly requested. Validate builds after any vendored-code edit.
