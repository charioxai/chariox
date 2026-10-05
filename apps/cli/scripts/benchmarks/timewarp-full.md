# MP-08 / MP-10 — WP-12 TimeWarp round 1

MP-08 / MP-10: owner pre-approved the complete official set on 2026-10-03.
Run the existing smoke runner with `--full` and the same environment variables.
The prompt remains byte-identical (including its historical smoke wording),
model gpt-6.1-sol, low effort, seed42, one repeat, 180s and 60 mutating actions.
One persistent provider thread; no score-driven analysis or tuning. Smoke rows
remain marked smoke. Full rows append incrementally and retain RED receipts.
MP-08 / MP-10: an infrastructure restart creates a new provider thread and must
be disclosed. Only explicit RED harness failures with no prompt submission or
agent action admit retry. Keep those original rows and use the latest permitted
attempt for canonical episode coverage; every acted-on or excluded row stays closed.

MP-08 / MP-10: freeze all231 official manifest goals ×six eras =1,386 episodes.
Order: Wiki/News tasks first, then Shop/cross-site, each by era and official
manifest order. Install unchanged official Shop source and the pinned 1,000
product fixture required by upstream's normal setup; never inspect reference
answers. A separate lane-owned2GiB/1CPU service container keeps Shop dependencies outside
the solver Room. Shop restarts each episode and its site random seed is42. Wiki/News
restart by era as in the smoke. Playwright is setup/evaluation only. The solver
uses only first-party Chariox Browser tools in its existing Room Chromium.

MP-08 / MP-10: pre-action setup recovery uses pip26.2 to expose dependency
conflicts promptly, public PyPI for CPU Torch build dependencies, and
python-dotenv1.1.1 to satisfy Pyserini1.3's fastmcp dependency (>=1.1.0).
Pin the observed spaCy3.8.16 and its normal official model3.8.0; curl retrieves
the hash-checked asset when pip receives GitHub HTTP503. Pyserini's eager,
unused OpenAI encoder constructor receives a non-secret placeholder with a
closed localhost endpoint; official Shop search uses local Lucene only.
These installation corrections precede every Shop solver action. Official
site and evaluator source remain unchanged. Stop after the first shared Shop
setup failure and preserve its unacted RED receipt before infrastructure repair.

MP-08 / MP-10: upstream's default judge and `gpt-5` alias resolve to
`gpt-5.1-2025-11-13`. Twelve episodes require that residual judge. Until an
approved access path is provisioned, explicitly exclude these episodes before
solver action, recording the reason and exact judge model. No substitute judge,
provider-credential export, or fabricated score. A partial deterministic result
cannot be labeled a complete official score.

MP-08 / MP-10: G2 priority means at most one Room per lane. The serial full
runner uses one Room and checks resource admission before Room and Shop setup;
the Shop service is separate and creates no additional solver Room or image.
The script keeps 30-minute status entries, exact
commands, resource samples, screenshot and tool audits. It creates no Rust
build, GitHub activity, deployment, or public leaderboard submission.

MP-08 / MP-10: source license metadata Apache-2.0, README MIT badge, official
data cards MIT, embedded WebShop MIT with Princeton NLP2023 notice. Local
execution is permitted; preserve notices and disclose conflicting declarations.
No single consistent redistribution license is asserted. Package results and
public source identities only; the owner decides whether to publish.
