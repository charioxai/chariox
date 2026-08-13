# Agent Slice Office Work Drills

These drills evaluate how far a Chariox agent in a slice can perform real
office work on behalf of a user with runtime MCP slice controls, extension
creation, and Chariox vault-backed credentials.

## 1. Email-Gated SaaS Onboarding

- Start a slice-backed agent with browser tools and vault tools.
- Store the Gmail credential through the Chariox vault.
- Ask the agent to log into Gmail using `paste_secret_to_slice`.
- Ask the agent to register for a service that requires email confirmation.
- The agent should find the confirmation email, complete onboarding, and report
  the created account metadata without exposing secrets.

Validation evidence:

- Slice screenshots for Gmail login, confirmation email, and completed service
  onboarding.
- Transcript check proving the password was not printed or returned.
- Vault handle list proving only credential metadata is visible.

## 2. Vendor Research To CRM-Like Entry

- Ask the agent to research three vendors for a concrete office purchase.
- Let it create or register a lightweight connector/script extension to normalize
  vendor data.
- Ask it to create an account on one vendor portal if required.
- The agent should compile a structured comparison and submit a contact/request
  form using vault-managed credentials where needed.

Validation evidence:

- Screenshots of vendor pages and submitted form.
- Extension registry output showing the created script or connector.
- Structured final comparison artifact.

## 3. Document Intake And Follow-Up Email

- Provide a document or web page with action items.
- Ask the agent to extract tasks, draft a response, and send a confirmation email
  from the Gmail account.
- If a new external service account is needed, the agent should create a generated
  vault credential and use it without reading the secret.

Validation evidence:

- Screenshot of draft/send state.
- Sent email visible in Gmail.
- Transcript check showing generated secret was never exposed.

## 4. Public API Workflow With Agent-Created Extension

- Ask the agent to find a public API or no-auth MCP useful for the task.
- The agent registers the MCP/script/connector extension, grants it to itself,
  and uses it in the same provider session.
- The agent combines browser work with extension output to complete a concrete
  business task, such as checking delivery status or market data.

Validation evidence:

- Extension registration and grant output.
- Tool call output from the newly registered extension.
- Screenshot or artifact proving the task result.

## 5. Support Ticket Lifecycle

- Ask the agent to register for a helpdesk or sandbox support portal.
- The agent creates a ticket, monitors email for confirmation, replies with an
  update, and records the ticket id in an artifact.
- If the portal requires a password, the agent uses a generated vault credential.

Validation evidence:

- Screenshots of ticket creation and email confirmation.
- Artifact with ticket id, account handle, and next steps.
- Transcript and vault checks proving secrets did not leak.
