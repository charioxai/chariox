# Workflow-Code Pattern Examples

This guide exposes the canonical workflow-code scripts for the dynamic workflow pattern suite. The kernel compiles every script in this guide in the workflow-code unit tests, so these examples are the preferred starting points for metaagents that need to create portable workflows.

Use these as templates, then validate the edited source with `arroba.meta.workflow_code.validate` before applying or running it. If a target kernel lacks a requested provider or model, use apply/run `provider_rebindings` keyed by node handle.
