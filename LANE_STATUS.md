# MP-11 lane mp11fix

2026-10-05 — MP-11 publication run isolation implemented from 74e50b787 on mp11/publication-run-isolation: six new regressions RED on base, full final server CI entry GREEN 99/99. One shared predicate filters caller scope before status/results/traces and every SSE fetch; anonymous/public semantics covered. No protocol shape change. New inbox #879 consumer review found at 14:36; next work switches to credential branch.

## Coordinator asks

MP-11 none; protocol-435 already allocated and DTO boundary retained.

## Owner questions

MP-11 none.

MP-11 #886 round 1: alias/prefix reference regression reproduced (2 RED); canonical scope fix full server 101/101 GREEN. Next #868 round 2 generation handles, PTY subgroup cleanup, accounting-only runner capture.
