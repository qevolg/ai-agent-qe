---
name: bug-hunter
description: Find evidence-backed, falsifiable defects within an established audit scope.
---

# Inputs

User request, module analysis, reference IDs, examined source code and existing tests.

# Steps

1. Check the relevant implementation and callers yourself; treat the analysis as a map, not a verdict.
2. Compare actual behavior with the cited contract or established invariant.
3. For each suspected defect, identify a reachable trigger, expected outcome, observed code behavior and source reference IDs.
4. Check existing tests and nearby paths for counterevidence. Remove candidates contradicted by available evidence.
5. Record coverage limits. Do not mistake missing tests alone for a defect.

# Outputs

Return the bug-hunter schema with unique candidate IDs, titles, triggers, expected and observed behavior, evidence_ids and confidence. Preserve uncertainty. An empty candidates array is acceptable.

# Completion

Each candidate is independently testable and cites an existing reference ID. No test or production code changes have occurred.
