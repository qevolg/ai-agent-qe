---
name: code-explorer
description: Interpret an audit request, inspect product or module code, and identify evidence and test execution requirements.
---

# Inputs

Original user prompt, base commit, project operational configuration, reference paths and the fetched worktree.

# Steps

1. Locate the requested product or module, read repository instructions, and identify relevant dependencies.
2. Trace entry points, call chains, state transitions, ownership and error handling. Identify examined and unexamined areas.
3. Assign reference IDs to concrete source locations and behavioral claims. Separate documented expectations, observed implementation and uncertain inference.
4. Find existing tests, build tasks, fixtures and Docker Compose definitions. Honor explicitly supplied environment settings.
5. Propose baseline argv commands and a runnable Compose environment with a persistent runner. The tool mounts the whole worktree read-only at `/workspace` and results at `/aaqe-results`. The runner must build into a writable output/cache directory. Record missing environment requirements rather than fabricating them.

# Outputs

Return the supplied code-explorer schema: summary, scope paths, source references, uncertainties, environment and baseline commands. Compose paths are relative to the worktree. Service definitions and business expectations come from the project; do not prescribe product-specific defaults. Use empty compose_files/runner_service if execution cannot yet be determined.

# Completion

Every claim has an identifiable source or an explicit uncertainty. The downstream stage can locate evidence and understand execution prerequisites. No files have changed.
